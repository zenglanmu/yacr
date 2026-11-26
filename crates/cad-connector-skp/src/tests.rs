//! Contract tests for the SketchUp preview adapter.
//!
//! Three layers:
//!
//! * unit tests for the release ceiling, version parsing and material mapping —
//!   they need no file at all;
//! * a **written-container round trip**: a textured model is serialized with the
//!   pinned 2026 writer to a real `.skp` under the system temp dir (deleted
//!   afterwards), then imported by [`SkpImporter`], so the §4v placement and the
//!   image entry are proved through the container rather than in memory. That is
//!   self-consistency (`skpwright` writing and reading its own output), not
//!   vendor acceptance;
//! * **env-gated** real-model tests. `skpwright`'s own evidence corpus is not
//!   redistributable here (its embedded textures are third-party content), so
//!   the samples stay outside the repository exactly like `YACR_TEST_DWG`: an
//!   unset variable returns early, and that skip is **not** evidence.
//!
//! ```text
//! YACR_TEST_SKP=/path/to/model.skp \
//! YACR_TEST_SKP_2026=/path/to/same-model-2026.skp \
//!   cargo test -p cad-connector-skp --locked -- --nocapture
//! ```

use cad_connector_core::{ImportLimits, MeshImport};
use cad_domain::{Completeness, DatabaseId, DocumentId, SemanticGeometry, SupportStatus, Unit};

use crate::convert::read_scene;
use crate::{
    ceiling_for, major_release, style_of, Importer, SkpImporter, DIAG_BACK_TEXTURE_NOT_IMPORTED,
    DIAG_DOCUMENT, DIAG_TEXTURES_NOT_SAMPLED, DIAG_TEXTURE_UNAVAILABLE, DIAG_UNITS_METRES,
    DIAG_UV_UNAVAILABLE,
};
use std::sync::Arc;

fn request(bytes: Vec<u8>) -> cad_connector_core::ImportRequest {
    cad_connector_core::ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

// ---- Release ceiling ------------------------------------------------------

#[test]
fn observed_releases_may_claim_verified() {
    for version in ["{3.0.1}", "{8.0.0}", "{13.0.0}", "{17.3.116}", "{20.0.0}"] {
        assert_eq!(
            ceiling_for(version, skpwright::Container::Stream),
            SupportStatus::Verified,
            "{version} has an observed corpus sample"
        );
    }
    for version in ["{26.2.0}", "{27.0.267}"] {
        assert_eq!(
            ceiling_for(version, skpwright::Container::Zip),
            SupportStatus::Verified,
            "{version} has an observed corpus sample"
        );
    }
}

#[test]
fn unobserved_releases_are_unverified_not_verified() {
    // 2021-2025 share the ZIP container but no sample was observed: container
    // equivalence is not evidence, so the ceiling is Unverified.
    for version in ["{21.0.0}", "{22.1.0}", "{23.0.0}", "{24.0.0}", "{25.0.0}"] {
        assert_eq!(
            ceiling_for(version, skpwright::Container::Zip),
            SupportStatus::Unverified,
            "{version} is read by container equivalence only"
        );
    }
    // Releases 9-12 sit between the two observed stream ranges.
    for version in ["{9.0.0}", "{12.0.0}"] {
        assert_eq!(
            ceiling_for(version, skpwright::Container::Stream),
            SupportStatus::Unverified
        );
    }
}

#[test]
fn an_unplaceable_header_is_unsupported() {
    assert_eq!(
        ceiling_for("{28.0.0}", skpwright::Container::Zip),
        SupportStatus::Unsupported
    );
    assert_eq!(
        ceiling_for("{17.0.0}", skpwright::Container::Zip),
        SupportStatus::Unsupported,
        "a stream-only release saved in the ZIP container is not a thing we model"
    );
    assert_eq!(
        ceiling_for("garbage", skpwright::Container::Stream),
        SupportStatus::Unsupported
    );
    assert_eq!(
        ceiling_for("{17.0.0}", skpwright::Container::Unknown),
        SupportStatus::Unsupported
    );
}

#[test]
fn a_version_string_is_parsed_inside_its_braces() {
    assert_eq!(major_release("{17.3.116}"), Some(17));
    assert_eq!(major_release("{3.0.1}"), Some(3));
    assert_eq!(major_release("26.2.0"), Some(26));
    assert_eq!(major_release(""), None);
    assert_eq!(major_release("{}"), None);
}

// ---- Material mapping -----------------------------------------------------

#[test]
fn a_solid_material_carries_its_colour_and_opacity() {
    let opaque = skpwright::Material::Solid {
        name: "Brick".into(),
        rgba: [180, 90, 40, 255],
        opacity: 1.0,
        colorize_type: 0,
    };
    assert_eq!(style_of(Some(&opaque)), (Some([180, 90, 40]), None));

    let glass = skpwright::Material::Solid {
        name: "Glass".into(),
        rgba: [200, 220, 240, 255],
        opacity: 0.48,
        colorize_type: 0,
    };
    let (colour, opacity) = style_of(Some(&glass));
    assert_eq!(colour, Some([200, 220, 240]));
    assert_eq!(opacity, Some(0.48));
}

#[test]
fn a_textured_material_falls_back_to_its_average_colour() {
    let textured = skpwright::Material::Textured {
        name: "Wood".into(),
        texture: Some("wood.jpg".into()),
        applied_size_in: None,
        jpeg_quality: None,
        image_bytes: None,
        avg_rgba: Some([120, 80, 40, 255]),
        colorized: false,
        texture_avg_rgba: Some([110, 70, 30, 255]),
        colorize_type: 0,
        opacity: 1.0,
    };
    assert_eq!(style_of(Some(&textured)), (Some([120, 80, 40]), None));

    let no_average = skpwright::Material::Textured {
        name: "Wood".into(),
        texture: None,
        applied_size_in: None,
        jpeg_quality: None,
        image_bytes: None,
        avg_rgba: None,
        colorized: false,
        texture_avg_rgba: None,
        colorize_type: 0,
        opacity: 1.0,
    };
    assert_eq!(
        style_of(Some(&no_average)),
        (Some([255, 255, 255]), None),
        "an unknown average stays neutral, it does not invent a colour"
    );
}

#[test]
fn a_missing_material_is_by_layer() {
    assert_eq!(style_of(None), (None, None));
}

// ---- Real-model evidence (env-gated, never a skip-as-evidence) -------------

#[derive(Debug, Default)]
struct Counted {
    meshes: usize,
    triangles: usize,
    vertices: usize,
    edges: usize,
    area: f64,
}

fn triangle_area(a: cad_domain::Point3, b: cad_domain::Point3, c: cad_domain::Point3) -> f64 {
    let ab = [b.x - a.x, b.y - a.y, b.z - a.z];
    let ac = [c.x - a.x, c.y - a.y, c.z - a.z];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() * 0.5
}

fn count(drawing: &cad_connector_core::ImportedDrawing) -> Counted {
    let mut counted = Counted::default();
    for entity in drawing.database.entities() {
        let (mesh, edges) = match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => (mesh, &[][..]),
            SemanticGeometry::Body3d(body) => (&body.display.mesh, &body.display.edges[..]),
            _ => continue,
        };
        counted.edges += edges.len();
        if !mesh.triangles.is_empty() {
            counted.meshes += 1;
            counted.triangles += mesh.triangles.len();
            counted.vertices += mesh.vertices.len();
            for vertex in &mesh.vertices {
                assert!(
                    vertex.x.is_finite() && vertex.y.is_finite() && vertex.z.is_finite(),
                    "imported vertices must be finite"
                );
            }
            for triangle in &mesh.triangles {
                counted.area += triangle_area(
                    mesh.vertices[triangle[0] as usize],
                    mesh.vertices[triangle[1] as usize],
                    mesh.vertices[triangle[2] as usize],
                );
            }
        }
    }
    counted
}

/// Import one real `.skp` when a path is supplied.
///
/// The sample is **not** committed (SketchUp's format has no redistributable
/// fixture with geometry available to this repository), so an unset variable
/// returns early and that skip is **not** evidence:
///
/// ```text
/// YACR_TEST_SKP=/path/to/model.skp cargo test -p cad-connector-skp real_sketchup -- --nocapture
/// ```
#[test]
fn a_real_sketchup_model_imports_when_supplied() {
    let Ok(path) = std::env::var("YACR_TEST_SKP") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("YACR_TEST_SKP points at a readable file");
    let byte_count = bytes.len();
    let drawing = match SkpImporter::new().import(&request(bytes), &|| false) {
        Ok(drawing) => drawing,
        Err(cad_domain::CadError::Unsupported(message)) => {
            // A sample may legitimately have no visible geometry at all, or be
            // carried by a release this reader does not model. Both are
            // **explicit** refusals with a stable code — never an empty success.
            // Edge-only files are no longer refused (ADR-0049): they import as
            // `Partial` wire bodies.
            eprintln!("refused: {message}");
            assert!(
                message.starts_with(crate::NO_GEOMETRY)
                    || message.starts_with(crate::VERSION_UNSUPPORTED),
                "an unsupported sample must be refused by a documented code: {message}"
            );
            return;
        }
        Err(other) => panic!("unexpected refusal for a real sample: {other:?}"),
    };
    let counted = count(&drawing);

    let mut codes: Vec<&str> = drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    codes.sort_unstable();
    eprintln!(
        "real SKP: bytes={byte_count} meshes={} triangles={} vertices={} area={:.6} completeness={} codes={codes:?}",
        counted.meshes,
        counted.triangles,
        counted.vertices,
        counted.area,
        drawing.report.completeness_label()
    );

    assert!(counted.meshes > 0, "a real model must yield preview meshes");
    assert!(counted.triangles > 0, "preview meshes must carry triangles");
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "a preview import is always Partial"
    );
    assert_eq!(drawing.units.source, Unit::Meter);
    assert!(codes.contains(&DIAG_UNITS_METRES));
    assert!(codes.contains(&DIAG_DOCUMENT));
    for capability in &drawing.report.capabilities {
        assert!(
            matches!(
                capability.measure,
                SupportStatus::Partial | SupportStatus::Unsupported
            ),
            "SKP measure is at most display-edge endpoint/midpoint snap, never Verified: {} ({:?})",
            capability.type_key,
            capability.measure
        );
        assert_ne!(capability.render, SupportStatus::Unsupported);
    }
}

/// The same model saved by two SketchUp generations must import to the same
/// geometry: the containers differ, the model does not.
///
/// ```text
/// YACR_TEST_SKP=/path/to/model-2017.skp \
/// YACR_TEST_SKP_2026=/path/to/model-2026.skp \
///   cargo test -p cad-connector-skp same_model -- --nocapture
/// ```
#[test]
fn the_same_model_in_two_containers_imports_alike() {
    let (Ok(old), Ok(new)) = (
        std::env::var("YACR_TEST_SKP"),
        std::env::var("YACR_TEST_SKP_2026"),
    ) else {
        return;
    };
    let old_bytes = std::fs::read(&old).expect("YACR_TEST_SKP points at a readable file");
    let new_bytes = std::fs::read(&new).expect("YACR_TEST_SKP_2026 points at a readable file");
    let old_drawing = SkpImporter::new()
        .import(&request(old_bytes), &|| false)
        .expect("the older container imports");
    let new_drawing = SkpImporter::new()
        .import(&request(new_bytes), &|| false)
        .expect("the newer container imports");
    let old_count = count(&old_drawing);
    let new_count = count(&new_drawing);

    eprintln!("pair: old={old_count:?} new={new_count:?}");
    assert_eq!(
        old_count.meshes, new_count.meshes,
        "the same model must yield the same entity count in both containers"
    );
    // Triangle *counts* may differ by a few triangles: the even-odd filler
    // decomposes y-bands from the stored ordinates, and the two containers
    // store the same vertices at different precision (inches vs metres). The
    // covered surface must not differ.
    let scale = old_count.area.abs().max(1.0);
    assert!(
        (old_count.area - new_count.area).abs() / scale < 1e-9,
        "surface area must match across containers: {} vs {}",
        old_count.area,
        new_count.area
    );
}

// ---- Explicit refusals ----------------------------------------------------

#[test]
fn a_file_that_is_not_a_sketchup_model_is_refused_by_name() {
    let error = SkpImporter::new()
        .import(&request(b"AC1032 not a sketchup file".to_vec()), &|| false)
        .err()
        .expect("a foreign file is refused");
    match error {
        cad_domain::CadError::CorruptData(message) => {
            assert!(
                message.contains(crate::NOT_A_SKETCHUP_FILE),
                "the refusal must carry the stable code, got: {message}"
            );
        }
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

// ---- Textured materials (ADR-0038 A3b) ----------------------------------

/// A run mesh over `faces`; the run's edges/curves are irrelevant here.
fn mesh_with(vertices: Vec<[f64; 3]>, faces: Vec<skpwright::MeshFace>) -> skpwright::Mesh {
    skpwright::Mesh {
        vertices,
        faces,
        ..Default::default()
    }
}

fn face(outer: Vec<u32>) -> skpwright::MeshFace {
    skpwright::MeshFace {
        outer,
        ..Default::default()
    }
}

/// Registration only sniffs the container, so a signature is what this
/// crate's tests need; nothing here decodes the image.
fn png_bytes() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(&[0x00, 0x11, 0x22, 0x33]);
    bytes
}

/// A 2 m × 2 m painted quad whose material 5 is textured by `image`.
fn textured_model(image: Option<Vec<u8>>, applied: Option<(f64, f64)>) -> skpwright::Model {
    let mut painted = face(vec![0, 1, 2, 3]);
    painted.front_material = Some(5);
    skpwright::Model {
        materials: vec![skpwright::Material::Textured {
            name: "Pavers".into(),
            texture: Some("pavers.jpg".into()),
            applied_size_in: applied,
            jpeg_quality: None,
            image_bytes: image,
            avg_rgba: Some([180, 150, 120, 255]),
            colorized: false,
            texture_avg_rgba: None,
            colorize_type: 0,
            opacity: 1.0,
        }],
        material_links: vec![(5, 0)],
        geometry: vec![skpwright::GeometryRun {
            mesh: mesh_with(
                vec![
                    [0.0, 0.0, 0.0],
                    [2.0, 0.0, 0.0],
                    [2.0, 2.0, 0.0],
                    [0.0, 2.0, 0.0],
                ],
                vec![painted],
            ),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn a_textured_front_face_carries_uvs_and_its_embedded_image() {
    let image = png_bytes();
    let model = textured_model(Some(image.clone()), Some((12.0, 12.0)));
    let mut import = MeshImport::new(&request(Vec::new()), "SkpPreviewMesh").expect("import");
    let scene = read_scene(&model, &ImportLimits::default(), &|| false, &mut import)
        .expect("the textured model walks");

    assert_eq!(scene.placed.len(), 1);
    let mesh = &scene.placed[0].mesh;
    assert_eq!(
        mesh.uvs.len(),
        mesh.vertices.len(),
        "one UV pair per emitted vertex"
    );
    // The contract is exactly `uv_xform(Front, applied_size).apply(world_m)`:
    // recompute it from `skpwright`'s own API for the same vertices.
    let face = &model.geometry[0].mesh.faces[0];
    let xform = face
        .uv_xform(skpwright::Side::Front, (12.0, 12.0))
        .expect("the §4v map is invertible");
    for (vertex, uv) in mesh.vertices.iter().zip(&mesh.uvs) {
        let want = xform.apply([vertex.x, vertex.y, vertex.z]);
        assert!(
            (uv[0] - want[0]).abs() < 1e-12 && (uv[1] - want[1]).abs() < 1e-12,
            "UV {uv:?} does not match uv_xform.apply({:?}) = {want:?}",
            [vertex.x, vertex.y, vertex.z]
        );
    }
    assert_eq!(scene.stats.textured_groups, 1);
    assert_eq!(scene.stats.untextured_textured_groups, 0);
    assert_eq!(scene.stats.textured_faces, 1);

    let key = mesh
        .texture
        .as_deref()
        .expect("the texture key is attached");
    let bytes = import
        .textures()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, bytes)| bytes)
        .expect("the key is registered with the import");
    assert_eq!(
        &bytes[..],
        &image[..],
        "the key addresses the material's own image bytes"
    );
}

#[test]
fn an_external_texture_stays_untextured_and_is_counted() {
    // `image_bytes: None` means the material names a file outside the model.
    let model = textured_model(None, Some((12.0, 12.0)));
    let mut import = MeshImport::new(&request(Vec::new()), "SkpPreviewMesh").expect("import");
    let scene = read_scene(&model, &ImportLimits::default(), &|| false, &mut import)
        .expect("the model walks");

    let mesh = &scene.placed[0].mesh;
    assert!(mesh.uvs.is_empty(), "no texture, no UV set");
    assert!(mesh.texture.is_none());
    assert_eq!(scene.stats.external_textures, 1);
    assert_eq!(scene.stats.untextured_textured_groups, 1);
    assert_eq!(scene.stats.textured_groups, 0);
    assert_eq!(
        import.textures().count(),
        0,
        "nothing enters the document's texture table"
    );
    // The group keeps the material's average colour, as before ADR-0038.
    assert_eq!(scene.placed[0].color, Some([180, 150, 120]));
}

#[test]
fn a_textured_back_material_is_counted_not_imported() {
    let mut model = textured_model(Some(png_bytes()), Some((12.0, 12.0)));
    let mut backs = face(vec![0, 1, 2, 3]);
    backs.back_material = Some(5);
    backs.normal = [0.0, 0.0, -1.0];
    model.geometry[0].mesh.faces.push(backs);

    let mut import = MeshImport::new(&request(Vec::new()), "SkpPreviewMesh").expect("import");
    let scene = read_scene(&model, &ImportLimits::default(), &|| false, &mut import)
        .expect("the model walks");

    assert_eq!(
        scene.stats.back_textured_faces, 1,
        "the back-painted face is reported, not imported"
    );
    // The front-painted face still carries its texture.
    assert_eq!(scene.stats.textured_groups, 1);
}

#[test]
fn a_textured_material_without_an_applied_size_keeps_no_uvs() {
    // No applied size anywhere: the §4v denominator is unknown, so the group
    // must not guess a tiling.
    let model = textured_model(Some(png_bytes()), None);
    let mut import = MeshImport::new(&request(Vec::new()), "SkpPreviewMesh").expect("import");
    let scene = read_scene(&model, &ImportLimits::default(), &|| false, &mut import)
        .expect("the model walks");

    let mesh = &scene.placed[0].mesh;
    assert!(mesh.uvs.is_empty());
    assert!(mesh.texture.is_none());
    assert_eq!(scene.stats.uv_unavailable, 1);
    assert_eq!(scene.stats.untextured_textured_groups, 1);
    assert_eq!(
        import.textures().count(),
        0,
        "an image no mesh could sample is not registered"
    );
}

// ---- Written-container round trip (2026 writer -> this importer) ----------
//
// The evidence above builds a `skpwright::Model` in memory, so it cannot catch
// a container-level gap: the writer dropping the §4v block or the image entry
// would still pass. These tests author a model, serialize it with the pinned
// 2026 writer, write those bytes to a real file, read them back and import
// them, so the assertion is about a `.skp` on disk, not a struct.
//
// Self-consistency only: `skpwright` writes and reads the container. No
// SketchUp/Trimble product was involved, which is not vendor acceptance.

/// A `(10, 10)` inch applied size is the §4v denominator, so a 10-inch square
/// puts the corners exactly on the 0..1 UV range.
const QUAD_SIDE_M: f64 = 0.254;

fn encode_png() -> Vec<u8> {
    // 2×2 RGBA with four distinct texels: a real PNG the sniffer accepts, and
    // one whose bytes cannot collide with any other fixture here.
    let pixels: [u8; 16] = [
        255, 0, 0, 255, // red
        0, 255, 0, 255, // green
        0, 0, 255, 255, // blue
        255, 255, 0, 255, // yellow
    ];
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(&pixels).expect("png pixels");
        writer.finish().expect("png trailer");
    }
    out
}

/// The §4v front placement for a `+Z` face: the identity map, so texture inches
/// equal face-local inches (u = +X, v = +Y for a horizontal face).
fn identity_face_texture() -> Box<skpwright::FaceTexture> {
    Box::new(skpwright::FaceTexture {
        front: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        back: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        front_extra: [0.0; 3],
        back_extra: [0.0; 3],
        front_pins: Vec::new(),
        back_pins: Vec::new(),
        flags: [1, 0],
    })
}

/// One 10-inch square in the XY plane, painted with `slot`, plus the four
/// boundary edges `skpwright` requires a face ring to have.
fn written_quad(
    slot: Option<u16>,
    texture: Option<Box<skpwright::FaceTexture>>,
) -> skpwright::GeometryRun {
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [QUAD_SIDE_M, 0.0, 0.0],
        [QUAD_SIDE_M, QUAD_SIDE_M, 0.0],
        [0.0, QUAD_SIDE_M, 0.0],
    ];
    let edges = [(0u32, 1u32), (1, 2), (2, 3), (3, 0)]
        .into_iter()
        .map(|(v0, v1)| skpwright::MeshEdge {
            v0,
            v1,
            ..Default::default()
        })
        .collect();
    skpwright::GeometryRun {
        def_index: None,
        mesh: skpwright::Mesh {
            vertex_pids: vec![0; vertices.len()],
            vertices,
            faces: vec![skpwright::MeshFace {
                outer: vec![0, 1, 2, 3],
                normal: [0.0, 0.0, 1.0],
                front_material: slot,
                texture,
                ..Default::default()
            }],
            edges,
            vertices_in: Vec::new(),
        },
        ..Default::default()
    }
}

/// The model the writer is handed: material slot `5` links to material `0`.
fn written_model(
    material: skpwright::Material,
    texture: Option<Box<skpwright::FaceTexture>>,
) -> skpwright::Model {
    skpwright::Model {
        materials: vec![material],
        material_links: vec![(5, 0)],
        geometry: vec![written_quad(Some(5), texture)],
        ..Default::default()
    }
}

fn textured_written_model(image: Vec<u8>) -> skpwright::Model {
    written_model(
        skpwright::Material::Textured {
            name: "RoundTripTexture".into(),
            texture: Some("round-trip.png".into()),
            applied_size_in: Some((10.0, 10.0)),
            jpeg_quality: None,
            image_bytes: Some(image),
            avg_rgba: Some([200, 40, 40, 255]),
            colorized: false,
            texture_avg_rgba: Some([200, 40, 40, 255]),
            colorize_type: 0,
            opacity: 1.0,
        },
        Some(identity_face_texture()),
    )
}

/// Serialize `model` with the pinned 2026 writer exactly as the exporter does
/// (`Document::from_model_with` with a fixed save time and seed, then `write`),
/// and prove the bytes are a real file by writing and re-reading them.
fn write_and_reread(model: &skpwright::Model) -> Vec<u8> {
    let document = skpwright::Document::from_model_with(
        model,
        skpwright::WriteEnv {
            // Same convention as `crate::export::export_env`: a fixed save
            // time, so the written bytes are reproducible.
            unix_time: 1_700_000_000,
            seed: 0x5941_4352_534b_5000,
        },
    )
    .expect("the writer accepts a textured model");
    let written = document.write().expect("the 2026 container serializes");
    assert!(!written.is_empty(), "the writer produced a non-empty file");

    // A unique real file under the system temp dir, removed on drop.
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "yacr-skp-texture-{}-{unique}.skp",
        std::process::id()
    ));
    struct TempFile(std::path::PathBuf);
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let file = TempFile(path);
    std::fs::write(&file.0, &written).expect("the written container reaches disk");
    let reread = std::fs::read(&file.0).expect("the written container reads back");
    assert_eq!(reread, written, "the file on disk is the writer's bytes");
    drop(file);
    reread
}

fn codes_of(drawing: &cad_connector_core::ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

/// The display mesh of a preview entity, whichever carrier it chose.
///
/// A written quad carries its face ring's four `MeshEdge`s, so ADR-0049 makes it
/// a body; a group without complete records stays a plain mesh.
fn preview_mesh(entity: &cad_db::DbEntity) -> &cad_domain::Mesh {
    match &entity.geometry {
        SemanticGeometry::Mesh(mesh) => mesh,
        SemanticGeometry::Body3d(body) => &body.display.mesh,
        other => panic!("unexpected preview geometry {other:?}"),
    }
}

#[test]
fn a_written_textured_model_imports_with_uvs_and_its_image() {
    let image = encode_png();
    let model = textured_written_model(image.clone());

    // 1. The writer produced a 2026-container file the pinned reader parses back.
    let bytes = write_and_reread(&model);
    let (version, guid) = skpwright::header_info(&bytes).expect("a fixed header");
    assert_eq!(
        major_release(&version),
        Some(26),
        "the writer authors the 2026 container, got {version}"
    );
    assert!(guid.is_none(), "the ZIP container carries no model GUID");
    assert_eq!(
        skpwright::detect_container(&bytes),
        skpwright::Container::Zip,
        "the written file is the ZIP/tagged-record container"
    );
    let parsed = skpwright::Model::parse(&bytes).expect("the pinned reader parses its own output");
    assert_eq!(parsed.geometry.len(), 1);
    let face = &parsed.geometry[0].mesh.faces[0];
    assert!(
        face.texture.is_some(),
        "the §4v placement survives the written container"
    );
    // The reader's slot table is 1-based, so material slot 5 is read back as 1.
    assert_eq!(face.front_material, Some(1));
    let applied = parsed
        .applied_size_of(1)
        .expect("the applied size survives the written container");
    assert_eq!(applied, (10.0, 10.0));
    match parsed.material_of(1) {
        Some(skpwright::Material::Textured {
            image_bytes: Some(stored),
            ..
        }) => assert_eq!(
            stored, &image,
            "the material's image entry is written byte for byte"
        ),
        other => panic!("expected a textured material with its image, got {other:?}"),
    }

    // 2. The importer reads those bytes (no host file access beyond this test's
    //    own temp file, which is already gone) and applies the §4v map itself.
    let drawing = SkpImporter::new()
        .import(&request(bytes), &|| false)
        .expect("the written file imports");
    assert_eq!(
        drawing.database.entities().count(),
        1,
        "one material group over one painted run is one preview entity"
    );
    let entity = drawing.database.entities().next().expect("the entity");
    let mesh = preview_mesh(entity);
    assert_eq!(
        mesh.uvs.len(),
        mesh.vertices.len(),
        "one UV pair per vertex"
    );

    // The contract is exactly `uv_xform(Front, applied).apply(world_m)`, taken
    // from the model read back out of the file rather than the one authored.
    let xform = face
        .uv_xform(skpwright::Side::Front, applied)
        .expect("the §4v map is invertible");
    for (vertex, uv) in mesh.vertices.iter().zip(&mesh.uvs) {
        let want = xform.apply([vertex.x, vertex.y, vertex.z]);
        assert!(
            (uv[0] - want[0]).abs() < 1e-9 && (uv[1] - want[1]).abs() < 1e-9,
            "UV {uv:?} does not match uv_xform.apply({:?}) = {want:?}",
            [vertex.x, vertex.y, vertex.z]
        );
    }
    // ... and, independent of `skpwright`, a 10-inch square with a (10, 10)
    // applied size maps its corners onto the unit square.
    for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
        assert!(
            mesh.uvs
                .iter()
                .any(|uv| (uv[0] - corner[0]).abs() < 1e-9 && (uv[1] - corner[1]).abs() < 1e-9),
            "corner {corner:?} missing from {:?}",
            mesh.uvs
        );
    }

    // 3. The PNG reached the document's texture table, addressed by the mesh.
    let stored: Vec<(&str, &Arc<[u8]>)> = drawing.database.textures().collect();
    assert_eq!(stored.len(), 1, "exactly one document asset");
    let (key, stored_bytes) = stored[0];
    assert_eq!(
        &stored_bytes[..],
        &image[..],
        "the asset is the material's own PNG"
    );
    assert_eq!(
        mesh.texture.as_deref(),
        Some(key),
        "the mesh names the asset it was given"
    );

    // 4. Nothing texture-related was lost, and the ceiling is the preview one.
    let codes = codes_of(&drawing);
    assert!(
        !codes.contains(&DIAG_TEXTURES_NOT_SAMPLED),
        "the texture was sampled; got {codes:?}"
    );
    assert!(
        !codes.contains(&DIAG_UV_UNAVAILABLE),
        "the face carried its §4v placement; got {codes:?}"
    );
    assert!(
        !codes.contains(&DIAG_TEXTURE_UNAVAILABLE),
        "the image is embedded, not external; got {codes:?}"
    );
    assert!(
        !codes.contains(&DIAG_BACK_TEXTURE_NOT_IMPORTED),
        "no face carried a back texture; got {codes:?}"
    );
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "preview geometry is always Partial, never Complete"
    );
}

#[test]
fn the_same_written_model_without_a_texture_reports_nothing_texture_related() {
    // Negative control: the same quad with a plain solid material and no §4v
    // placement is a solid-coloured preview mesh, and no texture diagnostic
    // may appear.
    let model = written_model(
        skpwright::Material::Solid {
            name: "Plain".into(),
            rgba: [30, 60, 90, 255],
            opacity: 1.0,
            colorize_type: 0,
        },
        None,
    );
    let bytes = write_and_reread(&model);
    let drawing = SkpImporter::new()
        .import(&request(bytes), &|| false)
        .expect("the written solid imports");

    assert_eq!(drawing.database.entities().count(), 1);
    let entity = drawing.database.entities().next().expect("the entity");
    let mesh = preview_mesh(entity);
    assert!(mesh.uvs.is_empty(), "a solid has no UV set");
    assert!(mesh.texture.is_none(), "a solid names no texture");
    assert_eq!(
        drawing.database.textures().count(),
        0,
        "an untextured model registers no document asset"
    );

    let codes = codes_of(&drawing);
    for code in [
        DIAG_TEXTURES_NOT_SAMPLED,
        DIAG_UV_UNAVAILABLE,
        DIAG_TEXTURE_UNAVAILABLE,
        DIAG_BACK_TEXTURE_NOT_IMPORTED,
    ] {
        assert!(
            !codes.contains(&code),
            "{code} does not belong to an untextured model; got {codes:?}"
        );
    }
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

mod edges;
mod sources;
