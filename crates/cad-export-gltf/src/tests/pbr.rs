//! PBR channel emission and its round trip (ADR-0039 D5).
//!
//! The written document is parsed back with `gltf::Gltf::from_slice` — the same
//! reader the importer uses — so every assertion is about the file's meaning, not
//! about the JSON this crate happened to build.

use super::*;

use cad_domain::{Completeness, MeshPbr, SupportStatus};
use gltf::Semantic;

/// `<32 lowercase hex>.png`, the only texture-key form the domain accepts.
const METALLIC_ROUGHNESS: &str = "11111111111111111111111111111111.png";
const NORMAL: &str = "22222222222222222222222222222222.png";
const EMISSIVE: &str = "33333333333333333333333333333333.png";
const OCCLUSION: &str = "44444444444444444444444444444444.png";

/// A real 1x1 RGB PNG (69 bytes), so every PBR image is an actual image rather
/// than a signature with filler behind it. One byte is appended per channel so
/// four distinct keys carry four distinct payloads.
fn png_bytes() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x38,
        0x21, 0xA7, 0x01, 0x00, 0x02, 0xC0, 0x01, 0x0F, 0xE4, 0xBD, 0x52, 0x15, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]
}

fn keyed_png(tag: u8) -> Vec<u8> {
    let mut bytes = png_bytes();
    bytes.push(tag);
    bytes
}

/// One triangle with UV0 and declared PBR channels.
fn pbr_mesh(pbr: MeshPbr) -> Mesh {
    Mesh {
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        pbr: Some(pbr),
        ..triangle_mesh()
    }
}

/// A PBR material naming all four slots, each with its own image.
fn full_pbr() -> MeshPbr {
    MeshPbr {
        metallic: 0.25,
        roughness: 0.75,
        emissive: [0.1, 0.2, 0.3],
        occlusion_strength: 0.6,
        metallic_roughness_texture: Some(METALLIC_ROUGHNESS.to_string()),
        normal_texture: Some(NORMAL.to_string()),
        emissive_texture: Some(EMISSIVE.to_string()),
        occlusion_texture: Some(OCCLUSION.to_string()),
    }
}

/// The database carrying those four images.
fn full_pbr_database(mesh: Mesh) -> DrawingDatabase {
    database_with_textures(
        vec![entity(1, "AcDbMesh", SemanticGeometry::Mesh(mesh))],
        Vec::new(),
        vec![
            (METALLIC_ROUGHNESS, keyed_png(1)),
            (NORMAL, keyed_png(2)),
            (EMISSIVE, keyed_png(3)),
            (OCCLUSION, keyed_png(4)),
        ],
    )
}

/// The single primitive of the single written mesh.
fn only_primitive<'a>(gltf: &'a gltf::Gltf) -> gltf::Primitive<'a> {
    gltf.meshes()
        .next()
        .expect("one mesh")
        .primitives()
        .next()
        .expect("one primitive")
}

#[test]
fn glb_writes_every_pbr_channel_and_its_images() {
    let database = full_pbr_database(pbr_mesh(full_pbr()));
    let output = export(GltfContainer::Glb, &database);

    assert!(
        matches!(output.report.completeness, Completeness::Complete),
        "every channel was writable: {:?}",
        output.report.completeness
    );
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.converted, 0);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = only_primitive(&gltf);
    assert!(
        primitive.get(&Semantic::TexCoords(0)).is_some(),
        "PBR textures need TEXCOORD_0"
    );

    let material = primitive.material();
    let metallic_roughness = material.pbr_metallic_roughness();
    assert_eq!(metallic_roughness.metallic_factor(), 0.25);
    assert_eq!(metallic_roughness.roughness_factor(), 0.75);
    assert_eq!(
        metallic_roughness.base_color_factor(),
        [1.0, 1.0, 1.0, 1.0],
        "no explicit entity colour: the base-colour factor stays neutral"
    );

    let normal = material.normal_texture().expect("normalTexture");
    assert_eq!(normal.scale(), 1.0, "the domain stores no separate scale");
    assert_eq!(normal.tex_coord(), 0);
    let occlusion = material.occlusion_texture().expect("occlusionTexture");
    assert_eq!(occlusion.strength(), 0.6);
    assert_eq!(occlusion.tex_coord(), 0);
    let emissive = material.emissive_texture().expect("emissiveTexture");
    assert_eq!(emissive.tex_coord(), 0);
    assert_eq!(material.emissive_factor(), [0.1, 0.2, 0.3]);

    // One image and one texture per distinct key, in writer order
    // (metallicRoughness, normal, emissive, occlusion), all in the one buffer.
    assert_eq!(gltf.images().count(), 4);
    assert_eq!(gltf.textures().count(), 4);
    assert_eq!(gltf.samplers().count(), 1);
    let slots = [
        metallic_roughness
            .metallic_roughness_texture()
            .unwrap()
            .texture(),
        normal.texture(),
        emissive.texture(),
        occlusion.texture(),
    ];
    for (position, texture) in slots.into_iter().enumerate() {
        let gltf::image::Source::View { view, mime_type } = texture.source().source() else {
            panic!("the image must live in the binary buffer");
        };
        assert_eq!(mime_type, "image/png");
        let data = &buffers[view.buffer().index()];
        assert_eq!(
            &data[view.offset()..view.offset() + view.length()],
            keyed_png(position as u8 + 1).as_slice(),
            "slot {position} must carry its own document bytes"
        );
    }
}

#[test]
fn pbr_none_writes_no_pbr_key_and_no_extra_bytes() {
    // The control: a mesh without PBR must produce exactly the pre-ADR-0039
    // document. `triangle_mesh()` never carried PBR, so its bytes are the control.
    let mesh = Mesh {
        // An explicit colour makes a material exist, which is where a stray PBR
        // key would show up most easily.
        ..triangle_mesh()
    };
    let database = database_with_textures(
        vec![entity(1, "AcDbMesh", SemanticGeometry::Mesh(mesh))],
        vec![(EntityId(1), explicit([255, 0, 0], None))],
        Vec::new(),
    );
    let output = export(GltfContainer::Gltf, &database);
    let json_text = String::from_utf8(output.bytes.clone()).expect("the .gltf body is JSON");
    for key in [
        "metallicFactor",
        "roughnessFactor",
        "metallicRoughnessTexture",
        "normalTexture",
        "occlusionTexture",
        "emissiveTexture",
        "emissiveFactor",
    ] {
        assert!(
            !json_text.contains(key),
            "{key} must not appear for a mesh with pbr: None"
        );
    }
    assert_eq!(
        output.report.counts.exact, 1,
        "a PBR-less mesh is still written exactly"
    );
    // The whole material is pinned: an explicit colour and nothing else. Any PBR
    // field would break this equality, which is the byte-shape guarantee.
    let json: serde_json::Value =
        serde_json::from_slice(&output.bytes).expect("the .gltf body is JSON");
    assert_eq!(
        json["materials"][0],
        serde_json::json!({
            "name": "entity_color_255_0_0",
            "pbrMetallicRoughness": { "baseColorFactor": [1.0, 0.0, 0.0, 1.0] },
        })
    );

    // The same database exported with a PBR channel differs, so the assertion
    // above is about the absent channels rather than about an inert writer.
    let with_pbr = database_with_textures(
        vec![entity(
            1,
            "AcDbMesh",
            SemanticGeometry::Mesh(pbr_mesh(MeshPbr {
                metallic: 0.5,
                roughness: 0.5,
                ..Default::default()
            })),
        )],
        Vec::new(),
        Vec::new(),
    );
    let pbr_json = String::from_utf8(export(GltfContainer::Gltf, &with_pbr).bytes)
        .expect("the .gltf body is JSON");
    assert!(pbr_json.contains("\"metallicFactor\":0.5"));
    assert!(
        !pbr_json.contains("normalTexture"),
        "a declared material writes only the channels it has"
    );
}

#[test]
fn two_meshes_with_different_pbr_get_two_materials() {
    let database = database_with_textures(
        vec![
            entity(
                1,
                "AcDbMesh",
                SemanticGeometry::Mesh(pbr_mesh(MeshPbr {
                    metallic: 0.0,
                    roughness: 0.5,
                    ..Default::default()
                })),
            ),
            entity(
                2,
                "AcDbMesh",
                SemanticGeometry::Mesh(Mesh {
                    vertices: vec![
                        point(5.0, 0.0, 0.0),
                        point(6.0, 0.0, 0.0),
                        point(5.0, 1.0, 0.0),
                    ],
                    ..pbr_mesh(MeshPbr {
                        metallic: 1.0,
                        roughness: 0.5,
                        ..Default::default()
                    })
                }),
            ),
        ],
        Vec::new(),
        Vec::new(),
    );
    let output = export(GltfContainer::Glb, &database);
    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(
        gltf.materials().count(),
        2,
        "a differing metallic factor must not share one material"
    );
    let mut metallic: Vec<f32> = gltf
        .meshes()
        .map(|mesh| {
            mesh.primitives()
                .next()
                .unwrap()
                .material()
                .pbr_metallic_roughness()
                .metallic_factor()
        })
        .collect();
    metallic.sort_by(f32::total_cmp);
    assert_eq!(metallic, vec![0.0, 1.0]);
}

#[test]
fn a_pbr_texture_without_bytes_is_reported_and_the_rest_is_written() {
    // The mesh names four keys the document does not carry; `cad-db` validation
    // refuses that shape, so this is a hand-built database — the writer must
    // still not invent an image.
    let mesh = pbr_mesh(full_pbr());
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    assert_eq!(
        output.report.counts.exact, 0,
        "a mesh that lost a channel is not exact"
    );
    let lost: Vec<&cad_import_core::CompatEntry> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.gltf.texture_not_written")
        .collect();
    assert_eq!(lost.len(), 4, "one conversion per unwritable channel");
    assert!(lost
        .iter()
        .all(|entry| entry.status == SupportStatus::Partial));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.images().count(), 0);
    let material = only_primitive(&gltf).material();
    let metallic_roughness = material.pbr_metallic_roughness();
    assert_eq!(
        metallic_roughness.metallic_factor(),
        0.25,
        "the scalars are written even when every image is missing"
    );
    assert!(metallic_roughness.metallic_roughness_texture().is_none());
    assert!(material.normal_texture().is_none());
    assert!(material.occlusion_texture().is_none());
    assert!(material.emissive_texture().is_none());
    assert_eq!(material.emissive_factor(), [0.1, 0.2, 0.3]);
}

#[test]
fn a_pbr_only_material_keeps_one_image_per_distinct_key() {
    // All four slots share one content key: one image, one texture, and one
    // material that samples it from all four channels.
    let shared = MeshPbr {
        metallic_roughness_texture: Some(METALLIC_ROUGHNESS.to_string()),
        normal_texture: Some(METALLIC_ROUGHNESS.to_string()),
        emissive_texture: Some(METALLIC_ROUGHNESS.to_string()),
        occlusion_texture: Some(METALLIC_ROUGHNESS.to_string()),
        occlusion_strength: 1.0,
        ..Default::default()
    };
    let database = database_with_textures(
        vec![entity(
            1,
            "AcDbMesh",
            SemanticGeometry::Mesh(pbr_mesh(shared)),
        )],
        Vec::new(),
        vec![(METALLIC_ROUGHNESS, png_bytes())],
    );
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.images().count(), 1);
    assert_eq!(gltf.textures().count(), 1);
    let material = only_primitive(&gltf).material();
    let source = |info: gltf::texture::Info<'_>| info.texture().source().index();
    assert_eq!(
        source(
            material
                .pbr_metallic_roughness()
                .metallic_roughness_texture()
                .unwrap()
        ),
        0
    );
    assert_eq!(
        material
            .normal_texture()
            .map(|normal| normal.texture().source().index()),
        Some(0)
    );
    assert_eq!(
        material
            .occlusion_texture()
            .map(|occlusion| occlusion.texture().source().index()),
        Some(0)
    );
    assert_eq!(
        material.emissive_texture().map(source),
        Some(0),
        "all four channels reach the one image"
    );
}

// ---------------------------------------------------------------------------
// Import -> export -> import.
// ---------------------------------------------------------------------------

/// The bytes shared by the round-trip fixture's single image.
fn round_trip_png() -> Vec<u8> {
    png_bytes()
}

/// A one-triangle PBR document with UV0 and one image behind every slot.
fn pbr_document() -> Vec<u8> {
    let png = round_trip_png();
    let mut bin = Vec::new();
    for position in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for component in position {
            bin.extend_from_slice(&component.to_le_bytes());
        }
    }
    let uvs_offset = bin.len();
    for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
        for component in uv {
            bin.extend_from_slice(&component.to_le_bytes());
        }
    }
    let indices_offset = bin.len();
    for index in [0u16, 1, 2] {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    let image_offset = bin.len();
    bin.extend_from_slice(&png);

    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},
          "indices":2,"material":0}}]}}],
        "materials":[{{"pbrMetallicRoughness":{{"metallicFactor":0.25,
          "roughnessFactor":0.75,"metallicRoughnessTexture":{{"index":0}}}},
          "normalTexture":{{"index":0}},
          "occlusionTexture":{{"index":0,"strength":0.6}},
          "emissiveTexture":{{"index":0}},
          "emissiveFactor":[0.1,0.2,0.3]}}],
        "textures":[{{"source":0}}],
        "images":[{{"bufferView":3,"mimeType":"image/png"}}],
        "accessors":[
          {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
            "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}},
          {{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}},
          {{"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}}
        ],
        "bufferViews":[
          {{"buffer":0,"byteOffset":0,"byteLength":36}},
          {{"buffer":0,"byteOffset":{uvs_offset},"byteLength":24}},
          {{"buffer":0,"byteOffset":{indices_offset},"byteLength":6}},
          {{"buffer":0,"byteOffset":{image_offset},"byteLength":{}}}
        ],
        "buffers":[{{"byteLength":{}}}]}}"#,
        png.len(),
        bin.len()
    );

    let mut json = json.as_bytes().to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
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

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn import_export_import_keeps_every_pbr_channel() {
    let first = import_bytes(pbr_document());
    let original = imported_mesh(&first)
        .pbr
        .clone()
        .expect("the fixture declares PBR");

    let output = export(GltfContainer::Glb, &first.database);
    assert!(
        matches!(output.report.completeness, Completeness::Complete),
        "the imported channels are all writable: {:?}",
        output.report.completeness
    );

    let second = import_bytes(output.bytes);
    let round_tripped = imported_mesh(&second)
        .pbr
        .clone()
        .expect("the export still declares PBR");

    assert_close(round_tripped.metallic, original.metallic);
    assert_close(round_tripped.roughness, original.roughness);
    for (actual, expected) in round_tripped.emissive.into_iter().zip(original.emissive) {
        assert_close(actual, expected);
    }
    assert_close(
        round_tripped.occlusion_strength,
        original.occlusion_strength,
    );
    // The keys are content addressed, so the same image returns the same key.
    for (actual, expected) in [
        (
            &round_tripped.metallic_roughness_texture,
            &original.metallic_roughness_texture,
        ),
        (&round_tripped.normal_texture, &original.normal_texture),
        (&round_tripped.emissive_texture, &original.emissive_texture),
        (
            &round_tripped.occlusion_texture,
            &original.occlusion_texture,
        ),
    ] {
        assert_eq!(
            actual, expected,
            "a channel key changed across the round trip"
        );
        assert!(actual.is_some(), "every channel of the fixture is present");
    }
}
