//! Contract tests for the SketchUp preview adapter.
//!
//! Two layers:
//!
//! * unit tests for the release ceiling, version parsing and material mapping —
//!   they need no file at all;
//! * **env-gated** real-model tests. `skpwright`'s own evidence corpus is not
//!   redistributable here (its embedded textures are third-party content), so
//!   the samples stay outside the repository exactly like `YACR_TEST_DWG`: an
//!   unset variable returns early, and that skip is **not** evidence.
//!
//! ```text
//! YACR_TEST_SKP=/path/to/model.skp \
//! YACR_TEST_SKP_2026=/path/to/same-model-2026.skp \
//!   cargo test -p cad-import-skp --locked -- --nocapture
//! ```

use cad_domain::{Completeness, DatabaseId, DocumentId, SemanticGeometry, SupportStatus, Unit};
use cad_import_core::ImportLimits;

use crate::{
    ceiling_for, major_release, style_of, Importer, SkpImporter, DIAG_DOCUMENT, DIAG_UNITS_METRES,
};
use std::sync::Arc;

fn request(bytes: Vec<u8>) -> cad_import_core::ImportRequest {
    cad_import_core::ImportRequest {
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

fn count(drawing: &cad_import_core::ImportedDrawing) -> Counted {
    let mut counted = Counted::default();
    for entity in drawing.database.entities() {
        if let SemanticGeometry::Mesh(mesh) = &entity.geometry {
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
/// YACR_TEST_SKP=/path/to/model.skp cargo test -p cad-import-skp real_sketchup -- --nocapture
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
            // A sample may legitimately be edge-only (SketchUp line work) or
            // carried by a release this reader does not model. Both are
            // **explicit** refusals with a stable code — never an empty success.
            eprintln!("refused: {message}");
            assert!(
                message.starts_with(crate::EDGE_ONLY_GEOMETRY)
                    || message.starts_with(crate::NO_GEOMETRY)
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
        assert_eq!(
            capability.measure,
            SupportStatus::Unsupported,
            "SKP preview geometry is never a MEASURE target: {}",
            capability.type_key
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
///   cargo test -p cad-import-skp same_model -- --nocapture
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
