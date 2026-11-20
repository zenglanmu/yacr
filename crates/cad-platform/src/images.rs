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
//!    references. It is deliberately **file-based only**: a mesh's base-colour
//!    texture is document state (`cad_db::DrawingDatabase::textures`), not a
//!    file reference (ADR-0038 D2/D3). [`has_document_textures`] is the cheap
//!    "any images at all?" companion for that second source.
//! 2. [`local::load_image_cache`] resolves each key under a drawing directory
//!    and an optional explicit images directory, reads the bytes, decodes them
//!    with `cad-resources` and inserts them into a [`DecodedImageCache`].
//! 3. [`local::load_image_cache_for_document`] is the same pass plus the
//!    document's own textures, decoded first from the database bytes with the
//!    same [`cad_resources::ResourceLimits`] and **no filesystem access**
//!    (ADR-0038 D10).
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

/// Whether the document owns any mesh base-colour texture bytes (ADR-0038 D3).
///
/// [`requested_images`] only reports file-based `Image` keys, so a drawing whose
/// only raster assets are mesh textures would otherwise look image-free and skip
/// the decode pass entirely. Callers that branch on "does this drawing reference
/// any image?" must OR this in: `requested_images(db).is_empty() &&
/// !has_document_textures(db)` is the real "no images at all" test.
///
/// Cheap (one map probe) and free of filesystem access: the bytes live in
/// `cad-db`.
pub fn has_document_textures(database: &DrawingDatabase) -> bool {
    database.textures().next().is_some()
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
        // A dimension's display children are recursed like a compound, so an
        // image inside a picture-block dimension is still collected (ADR 0017
        // A.8).
        SemanticGeometry::Dimension(dimension) => {
            for part in dimension.display() {
                collect_images(part, out);
            }
        }
        // A hatch is traversed like a compound (ADR 0018 E).
        SemanticGeometry::Hatch(hatch) => {
            for part in hatch.display() {
                collect_images(part, out);
            }
        }
        // A multileader is traversed like the compound it used to be
        // (ADR 0019 decision G).
        SemanticGeometry::MultiLeader(multileader) => {
            for part in multileader.display() {
                collect_images(part, out);
            }
        }
        // A table is traversed like the compound it used to be.
        SemanticGeometry::Table(table) => {
            for part in table.display() {
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
pub mod local;

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
            adjust: cad_domain::ImageAdjust::default(),
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

    /// The stored key is lowercased but the filesystem may be case-sensitive:
    /// `Textures/Logo.PNG` must resolve for a `textures/logo.png` request.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn case_insensitive_components_resolve_a_differently_cased_file() {
        use cad_resources::image_resource_key;
        use cad_resources::ResourceLimits;
        let dir = scratch("case-insensitive");
        std::fs::create_dir_all(dir.join("Textures")).unwrap();
        std::fs::write(
            dir.join("Textures").join("Logo.PNG"),
            encode_png(1, 1, &[7, 8, 9, 255]),
        )
        .unwrap();

        let (cache, report) = local::load_image_cache(
            &["textures/logo.png".to_string()],
            Some(&dir),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();
        assert_eq!(report.loaded, vec!["textures/logo.png".to_string()]);
        assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
        let decoded = cache
            .get(&image_resource_key("textures/logo.png").unwrap())
            .expect("the case-insensitive scan filled the lowercased key");
        assert_eq!(&decoded.rgba[0..4], &[7, 8, 9, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A control character or a `..` escape is never attempted.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn control_character_and_traversal_keys_are_rejected() {
        use cad_resources::ResourceLimits;
        let dir = scratch("unsafe-keys");
        for key in ["../secret.png", "a/\u{1}b.png", "a/../b.png"] {
            let (cache, report) = local::load_image_cache(
                &[key.to_string()],
                Some(&dir),
                None,
                &ResourceLimits::default(),
            )
            .unwrap();
            assert_eq!(report.unresolved, vec![key.to_string()], "{key:?}");
            assert!(report.loaded.is_empty(), "{key:?}");
            assert!(cache.is_empty(), "{key:?}");
        }
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

    // --- document-owned mesh textures (ADR-0038 D3/D10) ---------------------

    /// A canonical content-addressed key: 32 lowercase hex + `.png`.
    const TEXTURE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

    fn database_with_texture(key: &str, bytes: Vec<u8>) -> DrawingDatabase {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
        builder
            .insert_texture(key, std::sync::Arc::from(bytes))
            .expect("a canonical key with non-empty bytes");
        builder.finish().unwrap()
    }

    fn two_by_two_png() -> Vec<u8> {
        encode_png(
            2,
            2,
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 10, 20, 30, 255,
            ],
        )
    }

    #[test]
    fn has_document_textures_is_the_companion_to_requested_images() {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
        let empty = builder.finish().unwrap();
        assert!(requested_images(&empty).is_empty());
        assert!(!has_document_textures(&empty));

        let with_texture = database_with_texture(TEXTURE_KEY, two_by_two_png());
        assert!(
            requested_images(&with_texture).is_empty(),
            "a mesh texture is not a file reference"
        );
        assert!(
            has_document_textures(&with_texture),
            "the 'any images?' branch must fire for a texture-only document"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn document_textures_decode_into_the_cache_without_a_file() {
        use cad_resources::ResourceLimits;
        let db = database_with_texture(TEXTURE_KEY, two_by_two_png());
        // A drawing directory that does not exist proves nothing was read from
        // disk: the bytes are database state.
        let missing = std::path::Path::new("/definitely/not/a/drawing/dir");
        let (cache, report) = local::load_image_cache_for_document(
            &db,
            &[],
            Some(missing),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();

        assert_eq!(report.requested, vec![TEXTURE_KEY.to_string()]);
        assert_eq!(report.loaded, vec![TEXTURE_KEY.to_string()]);
        assert!(report.unresolved.is_empty());
        assert!(report.failed.is_empty());
        assert_eq!(report.bytes, 16, "2x2 RGBA");
        let decoded = cache
            .get(&cad_resources::ResourceKey(TEXTURE_KEY.to_string()))
            .expect("the mesh decode key is the batch's ResourceKey");
        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert_eq!(&decoded.rgba[0..4], &[255, 0, 0, 255]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_undecodable_document_texture_is_reported_by_key_and_left_out() {
        use cad_resources::ResourceLimits;
        let db = database_with_texture(TEXTURE_KEY, b"not really a png".to_vec());
        let (cache, report) =
            local::load_image_cache_for_document(&db, &[], None, None, &ResourceLimits::default())
                .expect("one bad texture must not fail the whole load");

        assert_eq!(report.requested, vec![TEXTURE_KEY.to_string()]);
        assert!(report.loaded.is_empty());
        assert!(report.unresolved.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(
            report.failed[0].starts_with(&format!("{TEXTURE_KEY}: ")),
            "the failure names the key, got {:?}",
            report.failed
        );
        assert!(
            cache.is_empty(),
            "no placeholder may stand in for an undecodable texture"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_over_budget_document_texture_is_reported_and_not_cached() {
        use cad_resources::ResourceLimits;
        let db = database_with_texture(TEXTURE_KEY, two_by_two_png());
        let limits = ResourceLimits {
            // A 2x2 RGBA payload is 16 bytes; one byte of budget refuses it.
            total_bytes: 1,
            ..ResourceLimits::default()
        };
        let (cache, report) =
            local::load_image_cache_for_document(&db, &[], None, None, &limits).unwrap();
        assert!(report.loaded.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].starts_with(&format!("{TEXTURE_KEY}: ")));
        assert!(cache.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_document_texture_wins_over_a_same_named_file_reference() {
        use cad_resources::{ResourceKey, ResourceLimits};
        let dir = scratch("doc-texture-wins");
        // A file under the exact same logical key, with different pixels.
        std::fs::write(dir.join(TEXTURE_KEY), encode_png(1, 1, &[9, 9, 9, 255])).unwrap();
        let db = database_with_texture(TEXTURE_KEY, two_by_two_png());

        let (cache, report) = local::load_image_cache_for_document(
            &db,
            &[TEXTURE_KEY.to_string()],
            Some(&dir),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();

        assert_eq!(
            report.requested,
            vec![TEXTURE_KEY.to_string()],
            "the key is requested once, as a document texture"
        );
        assert_eq!(report.loaded, vec![TEXTURE_KEY.to_string()]);
        let decoded = cache.get(&ResourceKey(TEXTURE_KEY.to_string())).unwrap();
        assert_eq!(
            (decoded.width, decoded.height),
            (2, 2),
            "the document bytes win over the same-named file"
        );
        assert_eq!(&decoded.rgba[0..4], &[255, 0, 0, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn without_document_textures_the_report_is_identical_to_the_file_only_pass() {
        use cad_resources::ResourceLimits;
        let dir = scratch("doc-texture-parity");
        std::fs::write(dir.join("logo.png"), encode_png(1, 1, &[1, 2, 3, 255])).unwrap();
        let requested = vec!["logo.png".to_string(), "../escape.png".to_string()];
        let db = database_with_images(); // no textures

        let (file_cache, file_report) =
            local::load_image_cache(&requested, Some(&dir), None, &ResourceLimits::default())
                .unwrap();
        let (doc_cache, doc_report) = local::load_image_cache_for_document(
            &db,
            &requested,
            Some(&dir),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();

        assert_eq!(
            doc_report, file_report,
            "the document-aware pass must not change the file-based report"
        );
        assert_eq!(doc_cache.len(), file_cache.len());
        assert!(!has_document_textures(&db));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
