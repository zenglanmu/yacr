//! Base-colour textures: `TEXCOORD_0`, `images`, `textures`, `samplers` and
//! `baseColorTexture` (ADR-0038 D11).

use super::*;

use cad_domain::{Completeness, SupportStatus};
use gltf::Semantic;

// ---------------------------------------------------------------------------
// Base-colour textures (ADR-0038 D11).
// ---------------------------------------------------------------------------

/// The only accepted texture key form: `<32 lowercase hex>.png`.
const TEXTURE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

/// A real 1x1 RGB PNG (69 bytes), so the bytes the writer embeds are an actual
/// image and not a signature with filler behind it.
fn png_bytes() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x38,
        0x21, 0xA7, 0x01, 0x00, 0x02, 0xC0, 0x01, 0x0F, 0xE4, 0xBD, 0x52, 0x15, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]
}

/// One triangle with UV0 and a named document texture.
fn textured_mesh(texture: &str) -> Mesh {
    Mesh {
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        texture: Some(texture.to_string()),
        ..triangle_mesh()
    }
}

/// One textured mesh whose bytes are in the database under `texture`.
fn textured_database(texture: &str) -> DrawingDatabase {
    database_with_textures(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(textured_mesh(texture)),
        )],
        Vec::new(),
        vec![(texture, png_bytes())],
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

/// The `texture_not_written` entry, which must exist exactly once.
fn texture_not_written(output: &ExportOutput) -> &cad_import_core::CompatEntry {
    let mut entries = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.gltf.texture_not_written");
    let entry = entries.next().expect("texture_not_written entry");
    assert!(
        entries.next().is_none(),
        "the loss is reported once per mesh"
    );
    entry
}

#[test]
fn glb_writes_uv0_and_one_base_colour_texture() {
    let database = textured_database(TEXTURE_KEY);
    let output = export(GltfContainer::Glb, &database);

    assert!(
        matches!(output.report.completeness, Completeness::Complete),
        "a writable texture is not a loss: {:?}",
        output.report.completeness
    );
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.converted, 0);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = only_primitive(&gltf);
    let positions = primitive.get(&Semantic::Positions).expect("POSITION");

    let uv = primitive.get(&Semantic::TexCoords(0)).expect("TEXCOORD_0");
    assert_eq!(
        uv.count(),
        positions.count(),
        "one UV per vertex, like POSITION"
    );
    assert_eq!(
        read_vec2_f32(&gltf, &buffers, uv.index()),
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        "UV0 is written verbatim, never frame-rotated"
    );

    assert_eq!(gltf.images().count(), 1, "one image per distinct key");
    let image = gltf.images().next().unwrap();
    let gltf::image::Source::View { view, mime_type } = image.source() else {
        panic!("the image must live in the binary buffer, not an external URI");
    };
    assert_eq!(mime_type, "image/png");
    let data = &buffers[view.buffer().index()];
    assert_eq!(
        &data[view.offset()..view.offset() + view.length()],
        &png_bytes()[..],
        "the image bufferView must carry the document bytes verbatim"
    );

    assert_eq!(gltf.textures().count(), 1);
    assert_eq!(gltf.samplers().count(), 1);
    let sampler = gltf.samplers().next().unwrap();
    assert_eq!(sampler.mag_filter(), Some(gltf::texture::MagFilter::Linear));
    assert_eq!(sampler.min_filter(), Some(gltf::texture::MinFilter::Linear));
    assert_eq!(sampler.wrap_s(), gltf::texture::WrappingMode::Repeat);
    assert_eq!(sampler.wrap_t(), gltf::texture::WrappingMode::Repeat);

    let pbr = primitive.material().pbr_metallic_roughness();
    let info = pbr.base_color_texture().expect("baseColorTexture");
    assert_eq!(info.tex_coord(), 0, "the texture reads TEXCOORD_0");
    assert_eq!(
        info.texture().source().index(),
        image.index(),
        "the material texture must point at the one image"
    );
    assert_eq!(
        pbr.base_color_factor(),
        [1.0, 1.0, 1.0, 1.0],
        "a texture without an entity colour keeps the neutral factor"
    );
}

#[test]
fn gltf_keeps_one_data_uri_buffer_and_embeds_the_image_in_it() {
    let database = textured_database(TEXTURE_KEY);
    let output = export(GltfContainer::Gltf, &database);
    assert!(!output.bytes.starts_with(b"glTF"));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid .gltf");
    assert_eq!(gltf.buffers().count(), 1, "the image joins the one buffer");
    let buffer = gltf.buffers().next().unwrap();
    let Source::Uri(uri) = buffer.source() else {
        panic!("a .gltf buffer must be the embedded data URI");
    };
    assert!(uri.starts_with(DATA_URI_PREFIX), "unexpected URI: {uri}");

    let buffers = buffer_bytes(&gltf);
    let image = gltf.images().next().expect("one image");
    let gltf::image::Source::View { view, mime_type } = image.source() else {
        panic!("the image must live in the embedded buffer");
    };
    assert_eq!(mime_type, "image/png");
    assert_eq!(view.buffer().index(), 0);
    assert_eq!(
        &buffers[0][view.offset()..view.offset() + view.length()],
        &png_bytes()[..]
    );

    let primitive = only_primitive(&gltf);
    let uv = primitive.get(&Semantic::TexCoords(0)).expect("TEXCOORD_0");
    assert_eq!(read_vec2_f32(&gltf, &buffers, uv.index()).len(), 3);
    assert_eq!(
        primitive
            .material()
            .pbr_metallic_roughness()
            .base_color_texture()
            .expect("baseColorTexture")
            .tex_coord(),
        0
    );
}

#[test]
fn uv0_without_a_texture_is_reported_and_not_written() {
    let mesh = Mesh {
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        texture: None,
        ..triangle_mesh()
    };
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    assert_eq!(output.report.counts.converted, 1);
    assert_eq!(
        output.report.counts.exact, 0,
        "a mesh that lost its UVs is not written exactly"
    );
    let entry = texture_not_written(&output);
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(
        entry.message.contains("#1"),
        "the entry must name the entity: {}",
        entry.message
    );

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let primitive = only_primitive(&gltf);
    assert!(
        primitive.get(&Semantic::TexCoords(0)).is_none(),
        "UVs without an image would claim an unsamplable parameterisation"
    );
    assert_eq!(gltf.images().count(), 0);
    assert_eq!(gltf.textures().count(), 0);
    assert_eq!(gltf.samplers().count(), 0);
}

#[test]
fn texture_bytes_that_do_not_match_the_key_extension_are_reported_and_skipped() {
    // A `.png` key with JPEG bytes: writing it as `image/png` would produce an
    // undecodable image inside a file that still claimed to be complete.
    let database = database_with_textures(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(textured_mesh(TEXTURE_KEY)),
        )],
        Vec::new(),
        vec![(
            TEXTURE_KEY,
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46],
        )],
    );
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    assert_eq!(output.report.counts.converted, 1);
    assert_eq!(output.report.counts.exact, 0);
    assert_eq!(texture_not_written(&output).status, SupportStatus::Partial);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.images().count(), 0);
    assert_eq!(gltf.textures().count(), 0);
    let primitive = only_primitive(&gltf);
    assert!(primitive.get(&Semantic::TexCoords(0)).is_none());
    assert!(primitive.material().index().is_none());
}

#[test]
fn two_meshes_sharing_a_texture_write_one_image_and_one_texture() {
    let database = database_with_textures(
        vec![
            entity(
                1,
                "GltfMesh",
                SemanticGeometry::Mesh(textured_mesh(TEXTURE_KEY)),
            ),
            // A different entity (and a different placement) naming the same
            // document texture must not duplicate its bytes.
            entity(
                2,
                "GltfMesh",
                SemanticGeometry::Mesh(Mesh {
                    vertices: vec![
                        point(5.0, 0.0, 0.0),
                        point(6.0, 0.0, 0.0),
                        point(5.0, 1.0, 0.0),
                    ],
                    ..textured_mesh(TEXTURE_KEY)
                }),
            ),
        ],
        Vec::new(),
        vec![(TEXTURE_KEY, png_bytes())],
    );
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.meshes().count(), 2);
    assert_eq!(gltf.images().count(), 1, "one image for the shared key");
    assert_eq!(gltf.textures().count(), 1);
    assert_eq!(
        gltf.materials().count(),
        1,
        "both primitives share the texture-only material"
    );

    let mut sources = Vec::new();
    for mesh in gltf.meshes() {
        let info = mesh
            .primitives()
            .next()
            .unwrap()
            .material()
            .pbr_metallic_roughness()
            .base_color_texture()
            .expect("every primitive is textured");
        assert_eq!(info.tex_coord(), 0);
        sources.push(info.texture().source().index());
    }
    assert_eq!(sources, vec![0, 0], "both textures reach the one image");
}

#[test]
fn an_explicit_entity_colour_and_a_texture_share_one_material() {
    // The entity colour stays the tint (baseColorFactor) and the texture is the
    // base colour; glTF multiplies the two, which is what the renderer does.
    let database = database_with_textures(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(textured_mesh(TEXTURE_KEY)),
        )],
        vec![(EntityId(1), explicit([255, 0, 0], Some(0.5)))],
        vec![(TEXTURE_KEY, png_bytes())],
    );
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.materials().count(), 1);
    let material = gltf.materials().next().unwrap();
    let pbr = material.pbr_metallic_roughness();
    assert_eq!(pbr.base_color_factor(), [1.0, 0.0, 0.0, 0.5]);
    assert_eq!(pbr.base_color_texture().expect("texture").tex_coord(), 0);
    assert_eq!(material.alpha_mode(), gltf::material::AlphaMode::Blend);
}

#[test]
fn vertex_colours_and_a_texture_write_both_the_attribute_and_the_material() {
    // `COLOR_0` is the tint and the material only exists for the texture, so the
    // base-colour factor stays neutral instead of doubling the colour.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..textured_mesh(TEXTURE_KEY)
    };
    let database = database_with_textures(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        Vec::new(),
        vec![(TEXTURE_KEY, png_bytes())],
    );
    let output = export(GltfContainer::Glb, &database);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let primitive = only_primitive(&gltf);
    assert!(primitive.get(&Semantic::Colors(0)).is_some());
    assert!(primitive.get(&Semantic::TexCoords(0)).is_some());
    let pbr = primitive.material().pbr_metallic_roughness();
    assert_eq!(pbr.base_color_factor(), [1.0, 1.0, 1.0, 1.0]);
    assert!(pbr.base_color_texture().is_some());
}

#[test]
fn an_untextured_export_has_no_texture_arrays_and_stays_deterministic() {
    let database = mesh_database(triangle_mesh());
    let first = export(GltfContainer::Glb, &database);
    let second = export(GltfContainer::Glb, &database);
    assert_eq!(
        first.bytes, second.bytes,
        "untextured export is deterministic"
    );

    // The `.gltf` spelling is the same document graph without the container, so
    // the absent keys can be asserted on the JSON directly.
    let json_output = export(GltfContainer::Gltf, &database);
    let json: serde_json::Value =
        serde_json::from_slice(&json_output.bytes).expect("the .gltf body is JSON");
    for key in ["images", "textures", "samplers"] {
        assert!(
            json.get(key).is_none(),
            "{key} must not exist when no mesh is textured"
        );
    }

    let gltf = gltf::Gltf::from_slice(&first.bytes).expect("valid GLB");
    let primitive = only_primitive(&gltf);
    assert!(primitive.get(&Semantic::TexCoords(0)).is_none());
    assert!(primitive.material().index().is_none());
    assert_eq!(gltf.accessors().count(), 2, "POSITION and indices only");
}
