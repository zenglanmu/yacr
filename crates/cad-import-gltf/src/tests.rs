//! Contract tests: hand-built GLB / `.gltf` bytes, frame conversion, honest
//! refusals and the connector descriptor. No external fixtures.

use std::sync::Arc;

use base64::Engine as _;
use cad_domain::{
    CadError, Completeness, DatabaseId, DocumentId, Point3, SemanticGeometry, SupportStatus,
    UnitContext,
};
use cad_import_core::{FidelityLevel, ImportLimits, ImportRequest, ImportedDrawing};

use super::*;

const TRIANGLE_PRIMITIVE: &str = r#"{"attributes":{"POSITION":0},"indices":1}"#;
const STRIP_PRIMITIVE: &str = r#"{"attributes":{"POSITION":0},"indices":1,"mode":5}"#;
const MATERIAL_PRIMITIVE: &str = r#"{"attributes":{"POSITION":0},"indices":1,"material":0}"#;
const COLORED_PRIMITIVE: &str = r#"{"attributes":{"POSITION":0,"COLOR_0":1},"indices":2}"#;
const COLORED_MATERIAL_PRIMITIVE: &str =
    r#"{"attributes":{"POSITION":0,"COLOR_0":1},"indices":2,"material":0}"#;

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

fn import(bytes: Vec<u8>) -> cad_domain::CadResult<ImportedDrawing> {
    GltfImporter::new().import(&request(bytes), &|| false)
}

/// Three f32 positions (0,0,0), (1,0,0), (0,1,0) followed by three u16
/// indices 0,1,2 — 36 + 6 = 42 bytes for buffer 0.
fn binary() -> Vec<u8> {
    let mut bytes = Vec::new();
    for position in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for component in position {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    assert_eq!(bytes.len(), 42);
    bytes
}

/// One scene, one node, one mesh whose primitives are supplied by the caller.
fn triangle_json(primitives: &str, buffer: &str) -> String {
    document_json(primitives, None, buffer)
}

/// Three f32 positions, three `VEC4` linear colours and three u16 indices —
/// 36 + 48 + 6 = 90 bytes for buffer 0.
fn colored_binary(colors: [[f32; 4]; 3]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for position in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for component in position {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    for color in colors {
        for component in color {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    assert_eq!(bytes.len(), 90);
    bytes
}

/// The same document shape as [`document_json`], with a linear `COLOR_0`
/// accessor (index 1) beside `POSITION` (index 0) and the indices (index 2).
fn color_document_json(primitives: &str, materials: Option<&str>) -> String {
    let materials = materials
        .map(|block| format!(",\n        \"materials\":[{block}]"))
        .unwrap_or_default();
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{primitives}]}}]{materials},
        "accessors":[
          {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
            "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}},
          {{"bufferView":1,"componentType":5126,"count":3,"type":"VEC4"}},
          {{"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}}
        ],
        "bufferViews":[
          {{"buffer":0,"byteOffset":0,"byteLength":36}},
          {{"buffer":0,"byteOffset":36,"byteLength":48}},
          {{"buffer":0,"byteOffset":84,"byteLength":6}}
        ],
        "buffers":[{{"byteLength":90}}]}}"#
    )
}

/// The same document, optionally carrying a JSON `materials` array.
fn document_json(primitives: &str, materials: Option<&str>, buffer: &str) -> String {
    let materials = materials
        .map(|block| format!(",\n        \"materials\":[{block}]"))
        .unwrap_or_default();
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{primitives}]}}]{materials},
        "accessors":[
          {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
            "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}},
          {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}
        ],
        "bufferViews":[
          {{"buffer":0,"byteOffset":0,"byteLength":36}},
          {{"buffer":0,"byteOffset":36,"byteLength":6}}
        ],
        "buffers":[{buffer}]}}"#
    )
}

/// A GLB container: 12-byte header, JSON chunk, BIN chunk.
fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut json = json.as_bytes().to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

fn meshes(drawing: &ImportedDrawing) -> Vec<&cad_domain::Mesh> {
    drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .collect()
}

fn codes(drawing: &ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

/// The render attributes of the single entity the document produced.
fn only_style(drawing: &ImportedDrawing) -> cad_db::EntityRenderAttributes {
    let entity = drawing
        .database
        .entities()
        .next()
        .expect("the document produced one entity");
    drawing.database.entity_render_attributes(entity.id)
}

#[test]
fn glb_triangle_imports_in_the_cad_frame() {
    let bytes = glb(
        &triangle_json(TRIANGLE_PRIMITIVE, r#"{"byteLength":42}"#),
        &binary(),
    );
    assert!(sniff(&bytes), "a version-2 GLB must be sniffed");

    let drawing = import(bytes).expect("the GLB imports");
    assert_eq!(drawing.database.entity_count(), 1);
    assert_eq!(drawing.units, UnitContext::from_meter());

    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let mesh = meshes[0];
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
    // glTF +Y up -> CAD +Z up: (0,1,0) becomes (0,0,1).
    assert_eq!(
        mesh.vertices,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0
            },
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0
            },
        ]
    );

    assert_eq!(drawing.report.completeness, Completeness::Complete);
    assert_eq!(drawing.report.connector, None);
    assert_eq!(drawing.vendor_records.len(), 0);

    let capability = &drawing.report.capabilities[0];
    assert_eq!(capability.type_key, GLTF_MESH_TYPE_KEY);
    assert_eq!(capability.read, SupportStatus::Verified);
    assert_eq!(capability.measure, SupportStatus::Unsupported);

    let codes = codes(&drawing);
    assert!(codes.contains(&DIAG_UNITS_METRES));
    assert!(codes.contains(&DIAG_FRAME_CONVERTED));
}

#[test]
fn data_uri_gltf_imports_the_same_geometry() {
    let payload = base64::engine::general_purpose::STANDARD.encode(binary());
    let buffer =
        format!(r#"{{"byteLength":42,"uri":"data:application/octet-stream;base64,{payload}"}}"#);
    let bytes = triangle_json(TRIANGLE_PRIMITIVE, &buffer).into_bytes();
    assert!(sniff(&bytes), "a JSON glTF object must be sniffed");

    let drawing = import(bytes).expect("the .gltf imports");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].vertices.len(), 3);
    assert_eq!(meshes[0].triangles, vec![[0, 1, 2]]);
    assert_eq!(
        meshes[0].vertices[2],
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0
        }
    );
}

#[test]
fn external_buffer_is_refused_by_name() {
    let buffer = r#"{"byteLength":42,"uri":"mesh.bin"}"#;
    let bytes = triangle_json(TRIANGLE_PRIMITIVE, buffer).into_bytes();
    match import(bytes) {
        Err(CadError::Unsupported(message)) => {
            assert!(
                message.contains(DIAG_EXTERNAL_BUFFER),
                "unexpected refusal message: {message}"
            );
            assert!(message.contains("mesh.bin"));
        }
        Err(other) => panic!("expected an explicit refusal, got {other:?}"),
        Ok(_) => panic!("expected an explicit refusal, got a successful import"),
    }
}

#[test]
fn document_without_a_scene_is_refused() {
    let bytes = br#"{"asset":{"version":"2.0"}}"#.to_vec();
    match import(bytes) {
        Err(CadError::Unsupported(message)) => {
            assert!(
                message.contains(DIAG_NO_SCENE),
                "unexpected refusal message: {message}"
            );
        }
        Err(other) => panic!("expected a no-scene refusal, got {other:?}"),
        Ok(_) => panic!("expected a no-scene refusal, got a successful import"),
    }
}

#[test]
fn triangle_strip_primitive_is_skipped_as_partial() {
    // A strip beside a triangle: the triangle is imported, the strip is a
    // reported loss and the completeness verdict is Partial.
    let primitives = format!("{TRIANGLE_PRIMITIVE},{STRIP_PRIMITIVE}");
    let bytes = glb(
        &triangle_json(&primitives, r#"{"byteLength":42}"#),
        &binary(),
    );
    let drawing = import(bytes).expect("the mixed document imports");

    assert_eq!(drawing.database.entity_count(), 1);
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "a skipped primitive must make the import partial"
    );
    assert!(codes(&drawing).contains(&DIAG_PRIMITIVE_MODE_UNSUPPORTED));
}

#[test]
fn descriptor_names_both_directions() {
    assert_eq!(DESCRIPTOR_GLTF.id, "gltf");
    assert_eq!(DESCRIPTOR_GLTF.extensions, &["gltf", "glb"]);
    assert!(DESCRIPTOR_GLTF.import.is_some());
    let export = DESCRIPTOR_GLTF.export.expect("the glTF writer is declared");
    assert_eq!(export.level, FidelityLevel::L1);
    assert!(export.lossy);
    assert_eq!(export.versions, "glTF 2.0 (GLB + JSON .gltf)");
}

#[test]
fn sniff_rejects_other_formats() {
    assert!(!sniff(b"o tri\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n"));
    let mut stl = vec![0u8; 80];
    stl.extend_from_slice(&0u32.to_le_bytes());
    assert!(!sniff(&stl));
    assert!(!sniff(b"AC1032\x00\x00\x00\x00"));
    assert!(!sniff(br#"{"format":"dxf"}"#));

    // A version-1 GLB is not glTF 2.0.
    let mut glb_v1 = b"glTF".to_vec();
    glb_v1.extend_from_slice(&1u32.to_le_bytes());
    glb_v1.extend_from_slice(&0u32.to_le_bytes());
    assert!(!sniff(&glb_v1));

    assert!(sniff(
        &triangle_json(TRIANGLE_PRIMITIVE, r#"{"byteLength":42}"#).into_bytes()
    ));
}

#[test]
fn a_primitive_without_a_material_keeps_the_layer_colour() {
    // `primitive.material()` invents the glTF default material (white) for a
    // primitive that names none; that is a format default, not an authored
    // colour, so it must not become an explicit white entity.
    let bytes = glb(
        &triangle_json(TRIANGLE_PRIMITIVE, r#"{"byteLength":42}"#),
        &binary(),
    );
    let drawing = import(bytes).expect("the GLB imports");

    let style = only_style(&drawing);
    assert_eq!(style.color, cad_db::EntityColor::ByLayer);
    assert_eq!(style.transparency, cad_db::EntityTransparency::ByLayer);
    assert!(!codes(&drawing).contains(&DIAG_MATERIAL_IGNORED));
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn vertex_colours_without_a_material_keep_their_authored_linear_value() {
    // No material means the glTF default factor `[1, 1, 1]`: the vertex colour
    // is stored as authored, and the entity keeps the layer colour.
    let bytes = glb(
        &color_document_json(COLORED_PRIMITIVE, None),
        &colored_binary([[0.5, 0.5, 0.5, 1.0]; 3]),
    );
    let drawing = import(bytes).expect("the GLB imports");

    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let expected = crate::scene::linear_to_srgb_byte(0.5);
    for color in &meshes[0].colors {
        for channel in color {
            let delta = i32::from(*channel) - i32::from(expected);
            assert!(
                delta.abs() <= 1,
                "linear 0.5 must store sRGB {expected}, got {color:?}"
            );
        }
    }
    assert_eq!(only_style(&drawing).color, cad_db::EntityColor::ByLayer);
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn vertex_colours_are_multiplied_by_the_material_factor() {
    // glTF 2.0: `COLOR_0` multiplies `baseColorFactor`. Linear 0.5 times linear
    // 0.5 is linear 0.25, encoded to sRGB exactly once.
    let material = r#"{"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.5,0.5,1.0]}}"#;
    let bytes = glb(
        &color_document_json(COLORED_MATERIAL_PRIMITIVE, Some(material)),
        &colored_binary([[0.5, 0.5, 0.5, 1.0]; 3]),
    );
    let drawing = import(bytes).expect("the GLB imports");

    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let expected = crate::scene::linear_to_srgb_byte(0.25);
    for color in &meshes[0].colors {
        for channel in color {
            let delta = i32::from(*channel) - i32::from(expected);
            assert!(
                delta.abs() <= 1,
                "linear 0.25 must store sRGB {expected}, got {color:?}"
            );
        }
    }
    // The factor is baked into `COLOR_0`, so no second colour channel is stored.
    assert_eq!(only_style(&drawing).color, cad_db::EntityColor::ByLayer);
    // Multiplying the factor in is the format's own composition, not a loss.
    assert_eq!(drawing.report.completeness, Completeness::Complete);
    assert!(!codes(&drawing).contains(&DIAG_MATERIAL_IGNORED));
}

#[test]
fn an_opaque_material_ignores_the_factor_alpha() {
    let material = r#"{"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.25,0.0,0.5]},
            "alphaMode":"OPAQUE"}"#;
    let bytes = glb(
        &document_json(MATERIAL_PRIMITIVE, Some(material), r#"{"byteLength":42}"#),
        &binary(),
    );
    let drawing = import(bytes).expect("the GLB imports");

    let style = only_style(&drawing);
    assert_eq!(
        style.color,
        cad_db::EntityColor::Explicit([188, 137, 0]),
        "the named material's RGB factor stays explicit"
    );
    assert_eq!(
        style.transparency,
        cad_db::EntityTransparency::Explicit(1.0),
        "alphaMode OPAQUE must ignore baseColorFactor[3]"
    );
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn a_blend_material_keeps_an_alpha_below_one() {
    let material = r#"{"pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,0.5]},
            "alphaMode":"BLEND"}"#;
    let bytes = glb(
        &document_json(MATERIAL_PRIMITIVE, Some(material), r#"{"byteLength":42}"#),
        &binary(),
    );
    let drawing = import(bytes).expect("the GLB imports");

    assert_eq!(
        only_style(&drawing).transparency,
        cad_db::EntityTransparency::Explicit(0.5)
    );
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn a_mask_material_is_imported_opaque_and_reported() {
    let material = r#"{"pbrMetallicRoughness":{"baseColorFactor":[1.0,1.0,1.0,0.5]},
            "alphaMode":"MASK","alphaCutoff":0.4}"#;
    let bytes = glb(
        &document_json(MATERIAL_PRIMITIVE, Some(material), r#"{"byteLength":42}"#),
        &binary(),
    );
    let drawing = import(bytes).expect("the GLB imports");

    assert_eq!(
        only_style(&drawing).transparency,
        cad_db::EntityTransparency::Explicit(1.0),
        "a cutout mask cannot be represented; the entity is opaque"
    );
    assert!(codes(&drawing).contains(&DIAG_ALPHA_MASK_NOT_IMPORTED));
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "an unrepresentable cutout must make the import partial"
    );
}

#[test]
fn a_non_2x_asset_version_is_refused_by_name() {
    let bytes = br#"{"asset":{"version":"1.0"}}"#.to_vec();
    match import(bytes) {
        Err(CadError::Unsupported(message)) => assert!(
            message.contains(DIAG_UNSUPPORTED_VERSION),
            "unexpected refusal message: {message}"
        ),
        Err(other) => panic!("expected a version refusal, got {other:?}"),
        Ok(_) => panic!("expected a version refusal, got a successful import"),
    }
    // A missing version is the same refusal, never a 2.0 assumption.
    match import(br#"{"asset":{}}"#.to_vec()) {
        Err(CadError::Unsupported(message)) => {
            assert!(message.contains(DIAG_UNSUPPORTED_VERSION))
        }
        Err(other) => panic!("expected a version refusal, got {other:?}"),
        Ok(_) => panic!("expected a version refusal, got a successful import"),
    }
}

#[test]
fn a_glb_with_a_non_2x_json_version_is_refused() {
    // The container header says version 2, but the JSON `asset.version` does
    // not: the embedded document is what decides.
    let json =
        triangle_json(TRIANGLE_PRIMITIVE, r#"{"byteLength":42}"#).replace("\"2.0\"", "\"1.0\"");
    let bytes = glb(&json, &binary());
    match import(bytes) {
        Err(CadError::Unsupported(message)) => assert!(
            message.contains(DIAG_UNSUPPORTED_VERSION),
            "unexpected refusal message: {message}"
        ),
        Err(other) => panic!("expected a version refusal, got {other:?}"),
        Ok(_) => panic!("expected a version refusal, got a successful import"),
    }
}
