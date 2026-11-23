//! Contract tests: hand-built GLB / `.gltf` bytes, frame conversion, honest
//! refusals and the connector descriptor. No external fixtures.

use std::sync::Arc;

use base64::Engine as _;
use cad_connector_core::{FidelityLevel, ImportLimits, ImportRequest, ImportedDrawing};
use cad_domain::{
    CadError, Completeness, DatabaseId, DocumentId, Point3, SemanticGeometry, SupportStatus,
    UnitContext,
};

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

/// The three canonical positions and their 0,1,2 index list.
fn position_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    for position in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for component in position {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    bytes
}

fn index_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes
}

/// `[u, v]` pairs as little-endian `f32` bytes (one `VEC2` accessor's payload).
fn uv_bytes(pairs: &[[f32; 2]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for pair in pairs {
        for component in pair {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    bytes
}

fn color_bytes(colors: [[f32; 4]; 3]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for color in colors {
        for component in color {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    bytes
}

/// Hand-built PNG bytes: the real container signature plus a payload.
///
/// `register_texture` sniffs the container and content-addresses the bytes, so the
/// payload only has to be recognisable and distinct — this is not a decodable
/// image, and nothing in the import path decodes it.
fn png() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(b"hand-built base-colour texture");
    bytes
}

/// Accumulates a BIN chunk and the matching `bufferViews` entries, so an offset
/// is computed once by the fixture instead of by hand in every test.
#[derive(Default)]
struct Bin {
    bytes: Vec<u8>,
    views: Vec<String>,
}

impl Bin {
    /// Append `bytes` as the next buffer view and return its index.
    fn view(&mut self, bytes: &[u8]) -> usize {
        let offset = self.bytes.len();
        self.bytes.extend_from_slice(bytes);
        self.views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{}}}"#,
            bytes.len()
        ));
        self.views.len() - 1
    }

    fn views_json(&self) -> String {
        self.views.join(",")
    }
}

/// Which image a fixture's `images[0]` describes.
enum TextureImage<'a> {
    /// These PNG bytes are appended to the BIN chunk and referenced by a view.
    Bin(&'a [u8]),
    /// The `images` entry to emit verbatim (a `data:` URI or an external path).
    Entry(String),
}

/// A one-primitive document whose material carries a base-colour texture.
///
/// The accessor order is fixed: `0` positions, `1` `TEXCOORD_0`, `2` indices and
/// — when `colors` is set — `3` `COLOR_0`. The buffer is declared without a URI;
/// a caller that wants a `.gltf` replaces it with an embedded one.
fn textured_triangle(colors: Option<[[f32; 4]; 3]>, image: TextureImage<'_>) -> (String, Vec<u8>) {
    let mut bin = Bin::default();
    let positions = bin.view(&position_bytes());
    let uvs = bin.view(&uv_bytes(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]));
    let indices = bin.view(&index_bytes());
    let mut attributes = format!(r#""POSITION":{positions},"TEXCOORD_0":{uvs}"#);
    let mut accessors = format!(
        r#"{{"bufferView":{positions},"componentType":5126,"count":3,"type":"VEC3",
            "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}},
           {{"bufferView":{uvs},"componentType":5126,"count":3,"type":"VEC2"}},
           {{"bufferView":{indices},"componentType":5123,"count":3,"type":"SCALAR"}}"#
    );
    if let Some(colors) = colors {
        let view = bin.view(&color_bytes(colors));
        attributes.push_str(&format!(r#","COLOR_0":{view}"#));
        accessors.push_str(&format!(
            r#",{{"bufferView":{view},"componentType":5126,"count":3,"type":"VEC4"}}"#
        ));
    }
    let image = match image {
        TextureImage::Bin(bytes) => {
            let view = bin.view(bytes);
            format!(r#"{{"bufferView":{view},"mimeType":"image/png"}}"#)
        }
        TextureImage::Entry(entry) => entry,
    };
    let bytes = bin.bytes.len();
    let views = bin.views_json();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{{"attributes":{{{attributes}}},"indices":{indices},
          "material":0}}]}}],
        "materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
        "textures":[{{"source":0}}],
        "images":[{image}],
        "accessors":[{accessors}],
        "bufferViews":[{views}],
        "buffers":[{{"byteLength":{bytes}}}]}}"#
    );
    (json, bin.bytes)
}

/// Replace the fixture's `buffer` entry with an embedded base64 `data:` URI.
fn with_data_uri_buffer(json: &str, bin: &[u8]) -> String {
    let payload = base64::engine::general_purpose::STANDARD.encode(bin);
    let declared = format!(r#"{{"byteLength":{}}}"#, bin.len());
    let embedded = format!(
        r#"{{"byteLength":{},"uri":"data:application/octet-stream;base64,{payload}"}}"#,
        bin.len()
    );
    assert!(
        json.contains(&declared),
        "the fixture must declare its buffer without a URI"
    );
    json.replace(&declared, &embedded)
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

#[test]
fn a_glb_base_colour_texture_is_imported_with_its_uvs() {
    let png = png();
    let (json, bin) = textured_triangle(None, TextureImage::Bin(&png));
    let drawing = import(glb(&json, &bin)).expect("the textured GLB imports");

    assert_eq!(drawing.database.entity_count(), 1);
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let mesh = meshes[0];
    // Authored values, in the authored order: UVs are surface parameters, so the
    // CAD frame conversion never touches them.
    assert_eq!(mesh.uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    let key = mesh
        .texture
        .clone()
        .expect("the base-colour texture is referenced");
    let bytes = drawing
        .database
        .texture(&key)
        .expect("the texture bytes belong to the document");
    assert_eq!(bytes.as_ref(), png.as_slice());

    // A texture that really was imported is not a loss in any form.
    let codes = codes(&drawing);
    assert!(!codes.contains(&DIAG_TEXTURE_NOT_IMPORTED));
    assert!(!codes.contains(&DIAG_UVS_DROPPED));
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn a_data_uri_image_imports_the_same_texture() {
    let png = png();
    let payload = base64::engine::general_purpose::STANDARD.encode(&png);
    let entry = format!(r#"{{"uri":"data:image/png;base64,{payload}"}}"#);
    let (json, bin) = textured_triangle(None, TextureImage::Entry(entry));
    let json = with_data_uri_buffer(&json, &bin);
    let drawing = import(json.into_bytes()).expect("the textured .gltf imports");

    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    let key = meshes[0]
        .texture
        .clone()
        .expect("the base-colour texture is referenced");
    let bytes = drawing
        .database
        .texture(&key)
        .expect("the texture bytes belong to the document");
    assert_eq!(bytes.as_ref(), png.as_slice());
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn an_external_image_keeps_uvs_and_reports_the_missing_bytes() {
    let entry = r#"{"uri":"tex.png"}"#.to_string();
    let (json, bin) = textured_triangle(None, TextureImage::Entry(entry));
    let drawing = import(glb(&json, &bin)).expect("the geometry still imports");

    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let mesh = meshes[0];
    assert_eq!(mesh.vertices.len(), 3, "geometry is kept");
    assert_eq!(
        mesh.uvs,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        "UVs are kept even though the bytes are unavailable"
    );
    assert_eq!(mesh.texture, None);
    assert!(drawing.database.textures().next().is_none());

    let codes = codes(&drawing);
    assert!(codes.contains(&DIAG_EXTERNAL_IMAGE));
    assert!(codes.contains(&DIAG_TEXTURE_NOT_IMPORTED));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_uv_count_mismatch_drops_the_uv_set_and_the_texture() {
    let png = png();
    let mut bin = Bin::default();
    let positions = bin.view(&position_bytes());
    // Four pairs for three vertices: the set cannot describe the geometry.
    let uvs = bin.view(&uv_bytes(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]));
    let indices = bin.view(&index_bytes());
    let image = bin.view(&png);
    let views = bin.views_json();
    let bytes = bin.bytes.len();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":{positions},"TEXCOORD_0":{uvs}}},
          "indices":{indices},"material":0}}]}}],
        "materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
        "textures":[{{"source":0}}],
        "images":[{{"bufferView":{image},"mimeType":"image/png"}}],
        "accessors":[
          {{"bufferView":{positions},"componentType":5126,"count":3,"type":"VEC3",
            "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}},
          {{"bufferView":{uvs},"componentType":5126,"count":4,"type":"VEC2"}},
          {{"bufferView":{indices},"componentType":5123,"count":3,"type":"SCALAR"}}
        ],
        "bufferViews":[{views}],
        "buffers":[{{"byteLength":{bytes}}}]}}"#
    );
    let drawing = import(glb(&json, &bin.bytes)).expect("the geometry still imports");

    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let mesh = meshes[0];
    assert_eq!(mesh.vertices.len(), 3, "the geometry is kept");
    assert!(mesh.uvs.is_empty(), "the UV set is dropped as a whole");
    assert_eq!(mesh.texture, None);
    // No orphan image: the texture was never registered.
    assert!(drawing.database.textures().next().is_none());

    assert!(codes(&drawing).contains(&DIAG_UVS_DROPPED));
    assert!(!codes(&drawing).contains(&DIAG_TEXTURE_NOT_IMPORTED));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_second_uv_set_is_noted_as_ignored() {
    let png = png();
    let mut bin = Bin::default();
    let positions = bin.view(&position_bytes());
    let uvs0 = bin.view(&uv_bytes(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]));
    // The material names set 0; this set is simply not the one it samples.
    let uvs1 = bin.view(&uv_bytes(&[[0.5, 0.5], [0.5, 0.5], [0.5, 0.5]]));
    let indices = bin.view(&index_bytes());
    let image = bin.view(&png);
    let views = bin.views_json();
    let bytes = bin.bytes.len();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":{positions},
          "TEXCOORD_0":{uvs0},"TEXCOORD_1":{uvs1}}},"indices":{indices},"material":0}}]}}],
        "materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
        "textures":[{{"source":0}}],
        "images":[{{"bufferView":{image},"mimeType":"image/png"}}],
        "accessors":[
          {{"bufferView":{positions},"componentType":5126,"count":3,"type":"VEC3",
            "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}},
          {{"bufferView":{uvs0},"componentType":5126,"count":3,"type":"VEC2"}},
          {{"bufferView":{uvs1},"componentType":5126,"count":3,"type":"VEC2"}},
          {{"bufferView":{indices},"componentType":5123,"count":3,"type":"SCALAR"}}
        ],
        "bufferViews":[{views}],
        "buffers":[{{"byteLength":{bytes}}}]}}"#
    );
    let drawing = import(glb(&json, &bin.bytes)).expect("the document imports");

    let meshes = meshes(&drawing);
    assert_eq!(
        meshes[0].uvs,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        "the set the material names is the one that is read"
    );
    assert!(meshes[0].texture.is_some());
    assert!(codes(&drawing).contains(&DIAG_UV_SET_IGNORED));
    // An ignored second set is a note, not a loss of this import.
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn a_textured_primitive_with_vertex_colours_keeps_both() {
    let png = png();
    let (json, bin) = textured_triangle(Some([[0.5, 0.25, 0.0, 1.0]; 3]), TextureImage::Bin(&png));
    let drawing = import(glb(&json, &bin)).expect("the GLB imports");

    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    let mesh = meshes[0];
    assert_eq!(mesh.uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    assert!(mesh.texture.is_some());
    assert_eq!(mesh.colors.len(), 3, "COLOR_0 is kept beside the texture");
    // `COLOR_0` still carries the composed colour; the texture multiplies into it
    // in the renderer, so it is not folded into the entity colour as well.
    assert_eq!(only_style(&drawing).color, cad_db::EntityColor::ByLayer);
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

// Sampler wrap: the renderer samples REPEAT, so any other declaration is named.
#[test]
fn a_non_repeat_sampler_wrap_is_named_not_reinterpreted() {
    let png = png();
    let (json, bin) = textured_triangle(None, TextureImage::Bin(&png));
    let json = with_sampler(&json, r#"{"wrapS":33071,"wrapT":33071}"#);
    let drawing = import(glb(&json, &bin)).expect("the textured primitive still imports");

    assert!(meshes(&drawing)[0].texture.is_some(), "the texture imports");
    assert!(
        codes(&drawing).contains(&DIAG_SAMPLER_WRAP_NOT_IMPORTED),
        "a declared ClampToEdge wrap must be reported, not silently treated as REPEAT"
    );
    let diagnostic = drawing
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DIAG_SAMPLER_WRAP_NOT_IMPORTED)
        .expect("the wrap diagnostic is present");
    assert!(
        diagnostic.message.contains("baseColorTexture")
            && diagnostic.message.contains("33071")
            && diagnostic.message.contains("REPEAT"),
        "the diagnostic must name the slot, the declared wrap and the REPEAT the \
         renderer actually samples: {}",
        diagnostic.message
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_repeat_or_absent_sampler_wrap_is_not_reported() {
    // glTF 2.0's default is REPEAT; a `{}` sampler and a document with no
    // `samplers` array both mean exactly what the renderer already does.
    let png = png();
    let (json, bin) = textured_triangle(None, TextureImage::Bin(&png));

    let empty_sampler = with_sampler(&json, "{}");
    let drawing = import(glb(&empty_sampler, &bin)).expect("an empty sampler imports");
    assert!(
        !codes(&drawing).contains(&DIAG_SAMPLER_WRAP_NOT_IMPORTED),
        "an empty sampler is glTF's REPEAT default and must not be reported"
    );
    assert_eq!(drawing.report.completeness, Completeness::Complete);

    // An explicit `10497` is the same declaration spelled out.
    let explicit_repeat = with_sampler(&json, r#"{"wrapS":10497,"wrapT":10497}"#);
    let drawing = import(glb(&explicit_repeat, &bin)).expect("an explicit REPEAT imports");
    assert!(!codes(&drawing).contains(&DIAG_SAMPLER_WRAP_NOT_IMPORTED));

    // No sampler at all: the default REPEAT, nothing to report.
    let drawing = import(glb(&json, &bin)).expect("a sampler-free document imports");
    assert!(!codes(&drawing).contains(&DIAG_SAMPLER_WRAP_NOT_IMPORTED));
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

/// Point `textures[0]` at sampler 0 and add the given `samplers` array.
fn with_sampler(json: &str, samplers: &str) -> String {
    let textures = r#""textures":[{"source":0}]"#;
    assert!(
        json.contains(textures),
        "the fixture must carry exactly one sampler-free texture"
    );
    json.replace(
        textures,
        &format!(r#""textures":[{{"sampler":0,"source":0}}],"samplers":[{samplers}]"#),
    )
}

// PBR channel cases (ADR-0039 D4), one per authored channel family.
mod pbr;
