//! Contract tests for the RVT preview lane.
//!
//! Unit 1 (`rvt` -> GLB) runs against the upstream `rvt` 0.22.0 scaffold
//! fixture: Apache-2.0, metadata-only, and honestly **zero** geometry. Unit 2
//! (GLB -> mesh) runs against a synthetic two-node GLB authored in this
//! repository; it is the test that proves node world transforms are applied.

use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;

use cad_domain::{CadError, DatabaseId, DocumentId, SupportStatus};
use cad_import_core::{detect_format, DrawingFormat, ImportLimits, ImportRequest};
use serde_json::{json, Value};

use super::*;
use crate::glb::{linear_to_srgb_byte, validate_factor};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/rvt")
        .join(name);
    std::fs::read(path).expect("RVT fixture is committed")
}

fn request(bytes: Vec<u8>) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

fn assert_mesh_invariants(mesh: &cad_domain::Mesh) {
    assert!(
        mesh.normals.is_empty() || mesh.normals.len() == mesh.vertices.len(),
        "normals must be empty or one per vertex"
    );
    assert!(
        mesh.colors.is_empty() || mesh.colors.len() == mesh.vertices.len(),
        "colours must be empty or one per vertex"
    );
    for triangle in &mesh.triangles {
        assert!(
            triangle
                .iter()
                .all(|index| (*index as usize) < mesh.vertices.len()),
            "triangle index is in range"
        );
    }
}

fn bounds(mesh: &cad_domain::Mesh) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for vertex in &mesh.vertices {
        min[0] = min[0].min(vertex.x);
        min[1] = min[1].min(vertex.y);
        min[2] = min[2].min(vertex.z);
        max[0] = max[0].max(vertex.x);
        max[1] = max[1].max(vertex.y);
        max[2] = max[2].max(vertex.z);
    }
    (min, max)
}

fn glb_fixture() -> Vec<u8> {
    fixture("synthetic-two-node.glb")
}

fn materials_glb_fixture() -> Vec<u8> {
    fixture("synthetic-materials.glb")
}

/// Find the placed mesh whose glTF node carries `name`.
fn placed<'a>(scene: &'a GlbScene, name: &str) -> &'a PlacedMesh {
    scene
        .meshes
        .iter()
        .find(|placed| placed.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the fixture has a node named {name:?}"))
}

/// Import and require a refusal. `ImportedDrawing` is not `Debug`, so
/// `expect_err` cannot be used on the importer's `Result`.
fn import_error(request: &ImportRequest) -> CadError {
    match RvtImporter::new().import(request, &|| false) {
        Ok(_) => panic!("expected an import refusal, got a drawing"),
        Err(error) => error,
    }
}

// ---- Real-model evidence (env-gated, never a skip-as-evidence) -------------

/// Import a real Autodesk sample project when one is supplied.
///
/// This mirrors the `YACR_TEST_DWG` convention: the file is **not** committed
/// (no public, redistributable Revit model with geometry exists), so an unset
/// variable returns early and that skip is **not** evidence. Supply a local
/// path to get real evidence:
///
/// ```text
/// YACR_TEST_RVT=fixtures/rvt/samples/racbasicsampleproject.rvt \
///   cargo test -p cad-import-rvt real_revit_project -- --nocapture
/// ```
#[test]
fn a_real_revit_project_imports_when_supplied() {
    let Ok(path) = std::env::var("YACR_TEST_RVT") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("YACR_TEST_RVT points at a readable file");
    let byte_count = bytes.len();
    let drawing = RvtImporter::new()
        .import(&request(bytes), &|| false)
        .expect("a real Revit project imports (preview)");

    let mut meshes = 0usize;
    let mut triangles = 0usize;
    let mut explicit_colors = 0usize;
    let mut by_layer_colors = 0usize;
    let mut explicit_opacity = 0usize;
    for entity in drawing.database.entities() {
        if let cad_domain::SemanticGeometry::Mesh(mesh) = &entity.geometry {
            meshes += 1;
            triangles += mesh.triangles.len();
            assert_mesh_invariants(mesh);
            let attributes = drawing.database.entity_render_attributes(entity.id);
            match attributes.color {
                EntityColor::Explicit(_) => explicit_colors += 1,
                _ => by_layer_colors += 1,
            }
            if matches!(attributes.transparency, EntityTransparency::Explicit(_)) {
                explicit_opacity += 1;
            }
        }
    }

    let mut codes: Vec<&str> = drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    codes.sort_unstable();

    eprintln!(
        "real RVT: bytes={byte_count} meshes={meshes} triangles={triangles} \
         explicit_colors={explicit_colors} by_layer_colors={by_layer_colors} \
         explicit_opacity={explicit_opacity} completeness={} codes={codes:?}",
        drawing.report.completeness_label()
    );

    // Preview geometry must be present: a zero-geometry container is refused
    // before this point, so reaching here with no mesh would be a false success.
    assert!(
        meshes > 0,
        "a real project must yield at least one preview mesh"
    );
    assert!(triangles > 0, "preview meshes must carry triangles");
    assert_eq!(
        explicit_colors + by_layer_colors,
        meshes,
        "every mesh stores exactly one colour provenance"
    );
    // Preview is never a measurement source.
    for capability in &drawing.report.capabilities {
        assert_eq!(
            capability.measure,
            SupportStatus::Unsupported,
            "RVT geometry is preview-only: {} must not claim measurability",
            capability.type_key
        );
    }
}

// ---- Unit 2: GLB -> mesh -------------------------------------------------

#[test]
fn detection_recognises_the_ole_magic_as_the_ole_container() {
    assert_eq!(
        detect_format(&fixture("upstream-metadata-scaffold-2024-no-geometry.rvt")),
        DrawingFormat::Ole
    );
    assert_eq!(
        detect_format(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1rest"),
        DrawingFormat::Ole
    );
    assert_eq!(
        DrawingFormat::Ole.label(),
        "OLE compound file (Revit or DGN V8)"
    );
    // The glTF intermediate is never a user-visible format.
    assert_eq!(
        detect_format(b"glTF\x02\x00\x00\x00"),
        DrawingFormat::Unknown
    );
}

#[test]
fn two_nodes_keep_distinct_world_bounds() {
    let scene = read_glb(&glb_fixture(), &ImportLimits::default(), &|| false)
        .expect("the synthetic GLB reads");
    assert_eq!(scene.meshes.len(), 2, "one entity per node/mesh instance");
    assert_eq!(scene.nodes_visited, 3, "mesh node, parent group, mesh node");

    for placed in &scene.meshes {
        assert_mesh_invariants(&placed.mesh);
    }
    let mut bounds: Vec<([f64; 3], [f64; 3])> =
        scene.meshes.iter().map(|m| bounds(&m.mesh)).collect();
    bounds.sort_by(|left, right| left.0[0].total_cmp(&right.0[0]));

    // block-a: matrix translation x = -1, half-width 0.5.
    assert!((bounds[0].0[0] + 1.5).abs() < 1e-9, "{:?}", bounds[0]);
    assert!((bounds[0].1[0] + 0.5).abs() < 1e-9, "{:?}", bounds[0]);
    // block-b: parent x = +0.5 + local x = +0.5.
    assert!((bounds[1].0[0] - 0.5).abs() < 1e-9, "{:?}", bounds[1]);
    assert!((bounds[1].1[0] - 1.5).abs() < 1e-9, "{:?}", bounds[1]);

    assert!(
        bounds[0].1[0] < bounds[1].0[0],
        "the two nodes must not stack at the origin: {bounds:?}"
    );
    // glTF +Y up -> CAD +Z up: the square lies in the CAD XY plane.
    for (min, max) in &bounds {
        assert!(
            min[2].abs() < 1e-9 && max[2].abs() < 1e-9,
            "{min:?} {max:?}"
        );
    }
}

#[test]
fn normals_are_rotated_and_normalised() {
    let scene = read_glb(&glb_fixture(), &ImportLimits::default(), &|| false).unwrap();
    for placed in &scene.meshes {
        assert_eq!(placed.mesh.normals.len(), placed.mesh.vertices.len());
        for normal in &placed.mesh.normals {
            assert!((normal.x).abs() < 1e-9);
            assert!((normal.y).abs() < 1e-9);
            assert!((normal.z - 1.0).abs() < 1e-9, "glTF +Y normal -> CAD +Z");
        }
    }
}

#[test]
fn face_limit_names_the_real_count() {
    let limits = ImportLimits {
        max_mesh_faces: Some(1),
        ..ImportLimits::default()
    };
    let error = read_glb(&glb_fixture(), &limits, &|| false)
        .expect_err("2 triangles are over a 1-face limit");
    match error {
        CadError::InvalidInput(message) => assert!(
            message.contains("2 triangle(s)") && message.contains("over the 1 face import limit"),
            "{message}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn face_index_limit_names_the_real_count() {
    let limits = ImportLimits {
        max_mesh_face_indices: Some(3),
        ..ImportLimits::default()
    };
    let error = read_glb(&glb_fixture(), &limits, &|| false)
        .expect_err("6 indices are over a 3-index limit");
    match error {
        CadError::InvalidInput(message) => assert!(
            message.contains("6 face corner indices") && message.contains("over the 3"),
            "{message}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn entity_limit_names_the_instance_count() {
    let limits = ImportLimits {
        max_entities: 1,
        ..ImportLimits::default()
    };
    let error = read_glb(&glb_fixture(), &limits, &|| false)
        .expect_err("2 instances are over a 1-entity limit");
    match error {
        CadError::InvalidInput(message) => assert!(
            message.contains("more than 1 drawable node/mesh instance"),
            "{message}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn cancellation_during_the_walk_is_honoured() {
    let error = read_glb(&glb_fixture(), &ImportLimits::default(), &|| true)
        .expect_err("a cancelled read must stop");
    assert!(matches!(error, CadError::Cancelled), "{error:?}");
}

#[test]
fn patch_a_position_to_nan_is_rejected() {
    let mut bytes = glb_fixture();
    let (json, mut bin) = glb_parts(&bytes);
    bin[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
    bytes = rebuild_glb(&json, &bin);
    let error = read_glb(&bytes, &ImportLimits::default(), &|| false)
        .expect_err("a non-finite vertex is rejected");
    match error {
        CadError::CorruptData(message) => assert!(message.contains(NON_FINITE_VERTEX), "{message}"),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn a_degenerate_triangle_is_dropped_and_counted() {
    let mut bytes = glb_fixture();
    let (json, mut bin) = glb_parts(&bytes);
    // First triangle starts at BIN offset 96 in the committed fixture; make it
    // repeat vertex 0 twice, leave the second triangle valid.
    bin[96..102].copy_from_slice(&[0, 0, 0, 0, 1, 0]);
    bytes = rebuild_glb(&json, &bin);
    let scene = read_glb(&bytes, &ImportLimits::default(), &|| false).unwrap();
    assert_eq!(scene.degenerate_triangles, 2, "both instances drop one");
    for placed in &scene.meshes {
        assert_eq!(placed.mesh.triangles.len(), 1);
    }
}

#[test]
fn a_sparse_accessor_is_refused_by_name() {
    let (mut json, bin) = glb_parts(&glb_fixture());
    json["accessors"][0]["sparse"] = json!({
        "count": 1,
        "indices": {"bufferView": 2, "componentType": 5123},
        "values": {"bufferView": 0}
    });
    let error = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect_err("sparse accessors are not implemented");
    match error {
        CadError::Unsupported(message) => assert!(message.contains(SPARSE_ACCESSOR), "{message}"),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn an_external_buffer_is_refused() {
    let (mut json, bin) = glb_parts(&glb_fixture());
    json["buffers"][0]["uri"] = json!("data:application/octet-stream;base64,AAAA");
    let error = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect_err("an external buffer is not fetched");
    match error {
        CadError::Unsupported(message) => assert!(message.contains(EXTERNAL_BUFFER), "{message}"),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn a_node_cycle_is_refused() {
    let (mut json, bin) = glb_parts(&glb_fixture());
    json["nodes"] = json!([{"name": "cycle", "children": [0]}]);
    json["scenes"] = json!([{"nodes": [0]}]);
    let error = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect_err("a cycle is refused");
    match error {
        CadError::CorruptData(message) => assert!(message.contains(NODE_CYCLE), "{message}"),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn a_glb_without_a_scene_is_refused() {
    let (mut json, bin) = glb_parts(&glb_fixture());
    json.as_object_mut().unwrap().remove("scene");
    json.as_object_mut().unwrap().remove("scenes");
    let error = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect_err("no scene is refused");
    match error {
        CadError::CorruptData(message) => assert!(message.contains(NO_SCENE), "{message}"),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

// ---- Unit 2: glTF material colours ---------------------------------------

#[test]
fn linear_base_colours_use_the_srgb_transfer_function() {
    // `rvt` linearises its sRGB materials (`src/ifc/gltf.rs:665-675`); this is
    // the exact inverse, not a `powf(1.0 / 2.2)` shortcut.
    assert_eq!(linear_to_srgb_byte(0.0), 0);
    assert_eq!(linear_to_srgb_byte(1.0), 255);
    // sRGB 128 encodes from linear 0.2158605. The often-quoted 0.2140 is
    // slightly below the midpoint (it encodes to 127.49) and rounds to 127.
    assert_eq!(linear_to_srgb_byte(0.215_860_5), 128);
    assert_eq!(linear_to_srgb_byte(0.2140), 127);
    // The linear toe: 0.0031308 is the largest linear value in the linear part.
    assert_eq!(linear_to_srgb_byte(0.003_130_8), 10);
    // Out-of-range inputs clamp instead of wrapping; callers reject them first.
    assert_eq!(linear_to_srgb_byte(-1.0), 0);
    assert_eq!(linear_to_srgb_byte(2.0), 255);
}

#[test]
fn an_exporter_chosen_opacity_is_stored_and_counted() {
    // The patched exporter (ADR-0035) draws a body whose type uses several
    // materials with its most transparent constituent's alpha and marks it
    // `extras.alphaSource = "rvt-rs"`. The reader still stores the opacity (it
    // is a real material's, applied to the whole body) but counts it, so the
    // import report can say the body's opacity was chosen rather than read.
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    json["materials"][1]["extras"] = json!({
        "colourSource": "rvt-rs",
        "alphaSource": "rvt-rs",
    });
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("the GLB with material extras reads");
    let blend = placed(&scene, "blend");
    match blend.opacity {
        Some(opacity) => assert!((opacity - 0.5).abs() < f32::EPSILON, "{opacity}"),
        None => panic!("a chosen BLEND opacity must still be stored"),
    }
    assert_eq!(scene.alpha_chosen, 1, "one mesh took a chosen opacity");

    // The committed fixture carries no extras: the same alpha is the file's
    // own, and nothing is counted as chosen.
    let plain = read_glb(&materials_glb_fixture(), &ImportLimits::default(), &|| {
        false
    })
    .expect("the committed materials GLB reads");
    assert_eq!(plain.alpha_chosen, 0);

    // A `file` alpha is never reported as chosen.
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    json["materials"][1]["extras"] = json!({"alphaSource": "file"});
    let file_alpha = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("the GLB with a file alpha reads");
    assert_eq!(file_alpha.alpha_chosen, 0);
}

#[test]
fn materials_become_srgb_colour_opacity_or_bylayer() {
    let scene = read_glb(&materials_glb_fixture(), &ImportLimits::default(), &|| {
        false
    })
    .expect("the synthetic materials GLB reads");
    assert_eq!(scene.meshes.len(), 4, "one entity per node/mesh instance");

    for placed in &scene.meshes {
        assert_mesh_invariants(&placed.mesh);
        // Colour is stored on the entity, never baked per vertex: the mesh
        // channel stays empty (`Mesh` has no UV/texture either).
        assert!(
            placed.mesh.colors.is_empty(),
            "{:?} must not bake a per-vertex colour",
            placed.name
        );
    }

    // sRGB (200, 100, 50) linearised by rvt and converted back byte-exactly.
    let opaque = placed(&scene, "opaque");
    assert_eq!(opaque.color, Some([200, 100, 50]));
    assert_eq!(
        opaque.opacity, None,
        "alpha 1.0 stays opaque, no explicit value"
    );

    // alpha 0.5 under alphaMode BLEND -> explicit opacity 0.5 (not transparency).
    let blend = placed(&scene, "blend");
    assert_eq!(blend.color, Some([30, 144, 255]));
    match blend.opacity {
        Some(opacity) => assert!((opacity - 0.5).abs() < f32::EPSILON, "{opacity}"),
        None => panic!("a BLEND material with alpha 0.5 must store an opacity"),
    }

    // No material at all: keep the current ByLayer behaviour, invent nothing.
    let plain = placed(&scene, "plain");
    assert_eq!(plain.color, None);
    assert_eq!(plain.opacity, None);

    // One mesh, two primitives, two materials: the merged entity keeps ByLayer
    // (picking one material would recolour the other primitive's triangles) and
    // the loss is counted instead of hidden.
    let conflict = placed(&scene, "conflict");
    assert_eq!(conflict.color, None);
    assert_eq!(conflict.opacity, None);
    assert_eq!(
        conflict.mesh.triangles.len(),
        4,
        "both primitives' geometry survives the merge"
    );
    assert_eq!(scene.material_losses.conflicting, 1);
    assert_eq!(scene.material_losses.index_out_of_range, 0);
    assert_eq!(scene.material_losses.non_finite, 0);
    assert_eq!(scene.material_losses.out_of_range, 0);
}

#[test]
fn an_out_of_range_material_index_is_refused_at_parse_without_panicking() {
    // `gltf` 1.4.1's `Primitive::material()` unwraps the index, so this reader
    // never calls it. `Gltf::from_slice` rejects the file up front instead:
    // a corrupt internal GLB is refused whole (same rule as a triangle index),
    // never silently degraded and never an abort.
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    json["meshes"][0]["primitives"][0]["material"] = json!(99);
    let error = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect_err("an out-of-range material index is refused, not guessed");
    match error {
        CadError::CorruptData(message) => {
            assert!(message.contains(GLB_INVALID), "{message}");
            assert!(message.contains("Index out of bounds"), "{message}");
        }
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn an_out_of_range_base_color_factor_keeps_bylayer() {
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] = json!([1.5, 0.0, 0.0, 1.0]);
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("an out-of-range factor is a loss, not a refusal");
    // Never clamped into a plausible red.
    assert_eq!(placed(&scene, "opaque").color, None);
    // Both primitives that name material 0 lose it (the opaque node and the
    // conflict mesh's first primitive), and the conflict is then reported too.
    assert_eq!(scene.material_losses.out_of_range, 2);
    assert_eq!(scene.material_losses.conflicting, 1);
}

#[test]
fn a_non_finite_base_color_factor_keeps_bylayer() {
    let mut losses = MaterialLosses::default();
    assert!(validate_factor([f32::NAN, 0.0, 0.0, 1.0], &mut losses).is_none());
    assert!(validate_factor([0.0, f32::INFINITY, 0.0, 1.0], &mut losses).is_none());
    assert_eq!(losses.non_finite, 2);
    assert_eq!(losses.out_of_range, 0);
    // A valid factor converts; alpha comes back untouched.
    let (color, alpha) = validate_factor([1.0, 1.0, 1.0, 0.25], &mut losses).unwrap();
    assert_eq!(color, [255, 255, 255]);
    assert_eq!(alpha, 0.25);
}

#[test]
fn material_losses_become_stable_diagnostics() {
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] = json!([1.5, 0.0, 0.0, 1.0]);
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .unwrap();

    let mut import = MeshImport::new(&request(Vec::new()), RVT_MESH_TYPE_KEY).unwrap();
    record_scene(&mut import, &scene);
    let outcome = import.finish().unwrap();
    let material: Vec<&cad_domain::Diagnostic> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DIAG_MATERIAL_IGNORED)
        .collect();
    assert_eq!(material.len(), 2, "one per loss reason: range and conflict");
    assert!(
        material
            .iter()
            .any(|diagnostic| diagnostic.message.contains("outside glTF's [0, 1] range")),
        "{:?}",
        material.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        material
            .iter()
            .any(|diagnostic| diagnostic.message.contains("different materials")),
        "the conflicting-materials case must be reported, not hidden"
    );
    // The colour loss does not change the completeness flag by itself; the
    // preview reason already makes every successful RVT import Partial.
    assert_eq!(outcome.completeness, cad_domain::Completeness::Complete);
}

#[test]
fn a_glb_without_materials_keeps_every_entity_bylayer() {
    // The original two-node fixture has no materials array at all.
    let scene = read_glb(&glb_fixture(), &ImportLimits::default(), &|| false).unwrap();
    for placed in &scene.meshes {
        assert_eq!(placed.color, None);
        assert_eq!(placed.opacity, None);
    }
    assert!(scene.material_losses.is_empty());
}

// ---- Unit 1: rvt -> GLB --------------------------------------------------

#[test]
fn the_rvt_scaffold_decodes_to_an_empty_glb_without_claiming_geometry() {
    let limits = ImportLimits::default();
    let decoded = decode_to_glb(
        fixture("upstream-metadata-scaffold-2024-no-geometry.rvt"),
        &limits,
        &|| false,
    )
    .expect("the upstream scaffold opens");
    assert!(!decoded.has_geometry);
    assert_eq!(decoded.building_elements, 0);
    assert_eq!(decoded.geometry_elements, 0);
    assert_eq!(decoded.confidence_level, "scaffold");
    assert_eq!(decoded.release_status, "partial", "2024 in rvt's matrix");
    assert!(decoded.glb.starts_with(b"glTF"));
    let scene = read_glb(&decoded.glb, &limits, &|| false).expect("the empty GLB parses");
    assert!(scene.meshes.is_empty(), "a scaffold has no drawable node");
}

#[test]
fn the_rvt_scaffold_import_is_an_explicit_refusal_not_an_empty_success() {
    let error = import_error(&request(fixture(
        "upstream-metadata-scaffold-2024-no-geometry.rvt",
    )));
    match error {
        CadError::Unsupported(message) => {
            assert!(message.contains(NO_GEOMETRY), "{message}");
            assert!(message.contains("metadata/scaffold"), "{message}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn progress_reports_reading_then_parsing_before_the_no_geometry_refusal() {
    // A successful RVT import needs a project with bodies, which no committed
    // fixture has. The reachable phases still prove the progress plumbing: the
    // refusal happens after Reading and Parsing, never with fabricated counts.
    let phases = std::cell::RefCell::new(Vec::new());
    let sink = |progress: ImportProgress| phases.borrow_mut().push(progress.phase);
    let result = RvtImporter::new().import_with_progress(
        &request(fixture("upstream-metadata-scaffold-2024-no-geometry.rvt")),
        &|| false,
        &sink,
    );
    assert!(result.is_err(), "the scaffold must refuse");
    assert_eq!(
        &*phases.borrow(),
        &[ImportPhase::Reading, ImportPhase::Parsing]
    );
}

#[test]
fn decode_cancellation_is_honoured_before_the_rvt_run() {
    let error = decode_to_glb(
        fixture("upstream-metadata-scaffold-2024-no-geometry.rvt"),
        &ImportLimits::default(),
        &|| true,
    )
    .expect_err("cancelled before decode");
    assert!(matches!(error, CadError::Cancelled));
}

// ---- Family refusal ------------------------------------------------------

#[test]
fn family_classification_uses_byte_signals() {
    assert_eq!(classify_document(false, 3, false), DocumentKind::Family);
    assert_eq!(classify_document(true, 3, false), DocumentKind::Family);
    assert_eq!(classify_document(false, 0, true), DocumentKind::Family);
    assert_eq!(classify_document(true, 0, false), DocumentKind::Project);
    // A project whose ProjectInformation stream is unreadable is not silently
    // declared a family; it is decoded and refused only if it has no bodies.
    assert_eq!(classify_document(false, 0, false), DocumentKind::Unknown);
    assert!(part_atom_declares_family_table("<entry><A:family>"));
    assert!(part_atom_declares_family_table("<a:family>"));
    assert!(!part_atom_declares_family_table("<entry><title>x</title>"));
}

#[test]
fn a_family_shaped_container_is_refused_by_name() {
    let bytes = family_container();
    assert_eq!(detect_format(&bytes), DrawingFormat::Ole);
    let error = import_error(&request(bytes));
    match error {
        CadError::Unsupported(message) => {
            assert!(message.contains(FAMILY_NO_GEOMETRY), "{message}");
            assert!(message.contains(".rfa/.rft"), "{message}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn an_ole_container_without_revit_streams_is_not_a_revit_file() {
    let bytes = cfb_with(&[("Contents", b"not revit")]);
    let error = import_error(&request(bytes));
    match error {
        CadError::CorruptData(message) => assert!(message.contains(NOT_A_REVIT_FILE), "{message}"),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

/// `PartAtom` is family metadata, not a project invariant.
///
/// The Autodesk Revit 2024 basic sample project
/// (`racbasicsampleproject.rvt`) contains `ProjectInformation`,
/// `Global`, `ElemTable`, `PartitionTable`, `History`, `Contents`, `Formats`,
/// `RevitPreview4.0`, `TransmissionData`, `BasicFileInfo`,
/// `DocumentIncrementTable` and `ContentDocuments` — and **no** `PartAtom`.
/// Requiring all twelve of `rvt`'s "invariant" streams rejected a genuine
/// Autodesk project. Missing any other stream stays fatal.
#[test]
fn a_project_without_part_atom_is_readable_but_a_missing_elem_table_is_not() {
    assert!(
        crate::decode::fatal_missing_streams(vec!["PartAtom"], true).is_empty(),
        "a project may omit PartAtom"
    );
    assert_eq!(
        crate::decode::fatal_missing_streams(vec!["PartAtom"], false),
        vec!["PartAtom"],
        "a family without PartAtom is still suspicious"
    );
    assert_eq!(
        crate::decode::fatal_missing_streams(vec!["PartAtom", "ElemTable"], true),
        vec!["ElemTable"],
        "only PartAtom is excused, on projects only"
    );
}

#[test]
fn oversized_input_is_an_invalid_input_with_counts() {
    let limits = ImportLimits {
        max_file_bytes: 8,
        ..ImportLimits::default()
    };
    let mut request = request(fixture("upstream-metadata-scaffold-2024-no-geometry.rvt"));
    request.limits = limits;
    let error = import_error(&request);
    match error {
        CadError::InvalidInput(message) => {
            assert!(message.contains("20480 bytes"), "{message}");
            assert!(message.contains("8 byte"), "{message}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

// ---- Status ceiling ------------------------------------------------------

#[test]
fn release_status_is_the_capability_ceiling() {
    assert_eq!(ceiling_for("verified"), SupportStatus::Verified);
    assert_eq!(ceiling_for("partial"), SupportStatus::Partial);
    assert_eq!(ceiling_for("experimental"), SupportStatus::Unverified);
    assert_eq!(ceiling_for("unknown"), SupportStatus::Unverified);
    assert_eq!(ceiling_for("unsupported"), SupportStatus::Unsupported);
    assert_eq!(ceiling_for("verified-ish"), SupportStatus::Unsupported);

    let mut capabilities = vec![EntityCapability {
        type_key: RVT_MESH_TYPE_KEY.into(),
        read: SupportStatus::Verified,
        semantic: SupportStatus::Partial,
        render: SupportStatus::Verified,
        pick: SupportStatus::Verified,
        measure: SupportStatus::Unsupported,
    }];
    apply_ceiling(&mut capabilities, SupportStatus::Partial);
    let capability = &capabilities[0];
    assert_eq!(capability.read, SupportStatus::Partial);
    assert_eq!(capability.semantic, SupportStatus::Partial);
    assert_eq!(capability.render, SupportStatus::Partial);
    assert_eq!(capability.pick, SupportStatus::Partial);
    assert_eq!(capability.measure, SupportStatus::Unsupported);

    apply_ceiling(&mut capabilities, SupportStatus::Unverified);
    assert_eq!(capabilities[0].read, SupportStatus::Unverified);
    assert_eq!(capabilities[0].render, SupportStatus::Unverified);
    assert_eq!(capabilities[0].pick, SupportStatus::Unverified);
    // Measure is never upgraded, even by an "experimental" ceiling.
    assert_eq!(capabilities[0].measure, SupportStatus::Unsupported);
}

#[test]
fn a_verified_ceiling_still_never_claims_measurability() {
    // The lane's only project fixture is a scaffold, so there is no successful
    // import to inspect. These are the two facts that make an empty success
    // impossible: the ceiling is applied row by row, and measure is fixed.
    let mut capabilities = vec![EntityCapability {
        type_key: RVT_MESH_TYPE_KEY.into(),
        read: SupportStatus::Verified,
        semantic: SupportStatus::Verified,
        render: SupportStatus::Verified,
        pick: SupportStatus::Verified,
        measure: SupportStatus::Verified,
    }];
    apply_ceiling(&mut capabilities, SupportStatus::Verified);
    assert_eq!(capabilities[0].read, SupportStatus::Verified);
    assert_eq!(capabilities[0].measure, SupportStatus::Unsupported);
}

// ---- Helpers -------------------------------------------------------------

/// Split a GLB into its JSON document and BIN chunk.
fn glb_parts(bytes: &[u8]) -> (Value, Vec<u8>) {
    assert!(bytes.starts_with(b"glTF"));
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json_end = 20 + json_len;
    let json: Value = serde_json::from_slice(&bytes[20..json_end]).expect("fixture JSON");
    let bin_len = u32::from_le_bytes(bytes[json_end..json_end + 4].try_into().unwrap()) as usize;
    let bin_start = json_end + 8;
    (json, bytes[bin_start..bin_start + bin_len].to_vec())
}

fn rebuild_glb(json: &Value, bin: &[u8]) -> Vec<u8> {
    let mut json_bytes = serde_json::to_vec(json).unwrap();
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut bin_bytes = bin.to_vec();
    while !bin_bytes.len().is_multiple_of(4) {
        bin_bytes.push(0);
    }
    let total = 12 + 8 + json_bytes.len() + 8 + bin_bytes.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E4942u32.to_le_bytes());
    out.extend_from_slice(&bin_bytes);
    out
}

/// A CFB with a family-shaped `PartAtom` and no `ProjectInformation` stream.
fn family_container() -> Vec<u8> {
    let atom = br#"<?xml version="1.0" encoding="utf-8"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:A="urn:schemas-autodesk-com:partatom">
  <title>synthetic family</title>
  <A:family><A:part><A:name>Type 1</A:name></A:part></A:family>
</entry>"#;
    cfb_with(&[
        ("BasicFileInfo", b""),
        ("Contents", b""),
        ("Formats/Latest", b""),
        ("Global/ContentDocuments", b""),
        ("Global/DocumentIncrementTable", b""),
        ("Global/ElemTable", b""),
        ("Global/History", b""),
        ("Global/Latest", b""),
        ("Global/PartitionTable", b""),
        ("PartAtom", atom),
        ("RevitPreview4.0", b""),
        ("TransmissionData", b""),
        ("Partitions/67", b""),
    ])
}

fn cfb_with(streams: &[(&str, &[u8])]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut compound = cfb::CompoundFile::create(&mut cursor).expect("create CFB");
        let mut created = std::collections::BTreeSet::new();
        for (name, _) in streams {
            let parts: Vec<&str> = name.split('/').collect();
            for depth in 1..parts.len() {
                let parent = format!("/{}", parts[..depth].join("/"));
                if created.insert(parent.clone()) {
                    compound.create_storage(&parent).expect("create storage");
                }
            }
        }
        for (name, data) in streams {
            let path = format!("/{name}");
            let mut stream = compound.create_stream(&path).expect("create stream");
            stream.write_all(data).expect("write stream");
        }
        compound.flush().expect("flush CFB");
    }
    cursor.into_inner()
}
