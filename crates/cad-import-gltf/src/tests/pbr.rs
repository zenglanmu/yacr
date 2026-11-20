//! Contract tests for the metallic-roughness channels (ADR-0039 D4): the five
//! authored channel families, the "declared, not defaulted" rule, and every
//! named loss a PBR slot can incur.
//!
//! One test per channel family keeps the thing that can regress separate: the
//! scalars, each of the four texture slots, and the `pbr: None` rule that keeps
//! a `baseColorFactor`-only asset unlit.

use super::*;

use cad_domain::{Completeness, MeshPbr};

/// A one-triangle document whose single material is the caller's JSON.
///
/// The accessor order is fixed: `0` positions, then the `TEXCOORD_n` sets
/// (`uv_sets` of them, all with the same corner UVs), then the indices, then one
/// view per image. Images are exposed as `textures[0..n]`, so a material snippet
/// refers to them by index.
fn pbr_document(material: &str, uv_sets: u32, images: &[TextureImage<'_>]) -> (String, Vec<u8>) {
    let mut bin = Bin::default();
    let positions = bin.view(&position_bytes());
    let mut uv_accessors = Vec::new();
    for _ in 0..uv_sets {
        uv_accessors.push(bin.view(&uv_bytes(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]])));
    }
    let indices = bin.view(&index_bytes());
    let mut image_entries = Vec::new();
    for image in images {
        let entry = match image {
            TextureImage::Bin(bytes) => {
                let view = bin.view(bytes);
                format!(r#"{{"bufferView":{view},"mimeType":"image/png"}}"#)
            }
            TextureImage::Entry(entry) => entry.clone(),
        };
        image_entries.push(entry);
    }

    let mut attributes = format!(r#""POSITION":{positions}"#);
    let mut accessors = format!(
        r#"{{"bufferView":{positions},"componentType":5126,"count":3,"type":"VEC3",
            "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}}"#
    );
    for (set, view) in uv_accessors.iter().enumerate() {
        attributes.push_str(&format!(r#","TEXCOORD_{set}":{view}"#));
        accessors.push_str(&format!(
            r#",{{"bufferView":{view},"componentType":5126,"count":3,"type":"VEC2"}}"#
        ));
    }
    accessors.push_str(&format!(
        r#",{{"bufferView":{indices},"componentType":5123,"count":3,"type":"SCALAR"}}"#
    ));

    let (textures, images) = if image_entries.is_empty() {
        (String::new(), String::new())
    } else {
        let sources: Vec<String> = (0..image_entries.len())
            .map(|source| format!(r#"{{"source":{source}}}"#))
            .collect();
        (
            format!(r#","textures":[{}]"#, sources.join(",")),
            format!(r#","images":[{}]"#, image_entries.join(",")),
        )
    };
    let bytes = bin.bytes.len();
    let views = bin.views_json();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{{"attributes":{{{attributes}}},"indices":{indices},
          "material":0}}]}}],
        "materials":[{material}]{textures}{images},
        "accessors":[{accessors}],
        "bufferViews":[{views}],
        "buffers":[{{"byteLength":{bytes}}}]}}"#
    );
    (json, bin.bytes)
}

/// Import a PBR document and hand back the drawing plus its single mesh.
fn import_pbr(material: &str, uv_sets: u32, images: &[TextureImage<'_>]) -> ImportedDrawing {
    let (json, bin) = pbr_document(material, uv_sets, images);
    import(glb(&json, &bin)).expect("the PBR document imports")
}

fn only_pbr(drawing: &ImportedDrawing) -> MeshPbr {
    let meshes = meshes(drawing);
    assert_eq!(meshes.len(), 1, "the document produces one mesh");
    meshes[0]
        .pbr
        .clone()
        .expect("the material declared PBR, so the mesh carries it")
}

/// A PBR scalar in f64, compared within the `f32` the source format stores.
fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected}, got {actual}"
    );
}

/// A PBR colour component in f64, compared within the `f32` the source stores.
fn assert_rgb_close(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert_close(actual, expected);
    }
}

/// The document-owned bytes a PBR slot names.
fn texture_bytes<'a>(drawing: &'a ImportedDrawing, key: &str) -> &'a [u8] {
    drawing
        .database
        .texture(key)
        .expect("the PBR texture bytes belong to the document")
        .as_ref()
}

// ---------------------------------------------------------------------------
// The declared scalars.
// ---------------------------------------------------------------------------

#[test]
fn metallic_and_roughness_factors_are_imported() {
    let material = r#"{"pbrMetallicRoughness":{"metallicFactor":0.25,"roughnessFactor":0.75}}"#;
    let drawing = import_pbr(material, 0, &[]);

    let pbr = only_pbr(&drawing);
    assert_close(pbr.metallic, 0.25);
    assert_close(pbr.roughness, 0.75);
    assert_eq!(
        pbr.occlusion_strength, 1.0,
        "no occlusion map means the neutral identity"
    );
    assert_rgb_close(pbr.emissive, [0.0, 0.0, 0.0]);
    assert_eq!(pbr.metallic_roughness_texture, None);
    assert_eq!(pbr.normal_texture, None);
    assert_eq!(pbr.emissive_texture, None);
    assert_eq!(pbr.occlusion_texture, None);
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn a_base_colour_only_material_stays_pbr_none() {
    // ADR-0039 D1: glTF's default `metallicFactor = 1` is a spec default, not
    // authored data. Applying it would draw this asset as black metal.
    let material = r#"{"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.25,0.0,1.0],
        "baseColorTexture":{"index":0}}}"#;
    let png = png();
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&png)]);

    let meshes = meshes(&drawing);
    assert_eq!(
        meshes[0].pbr, None,
        "baseColorFactor/baseColorTexture alone never creates a MeshPbr"
    );
    assert!(meshes[0].texture.is_some(), "the base colour still imports");
    assert!(!codes(&drawing).contains(&DIAG_PBR_IGNORED));
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

// ---------------------------------------------------------------------------
// The four texture slots.
// ---------------------------------------------------------------------------

#[test]
fn a_metallic_roughness_texture_is_imported_with_its_uvs() {
    let bytes = png();
    let material = r#"{"pbrMetallicRoughness":{"roughnessFactor":0.5,"metallicRoughnessTexture":{"index":0}}}"#;
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&bytes)]);

    let mesh = meshes(&drawing)[0];
    assert_eq!(
        mesh.uvs,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        "a PBR texture makes set 0 the imported UV set"
    );
    let pbr = only_pbr(&drawing);
    let key = pbr
        .metallic_roughness_texture
        .expect("the metallic-roughness map is referenced");
    assert_eq!(texture_bytes(&drawing, &key), bytes.as_slice());
    assert_close(pbr.roughness, 0.5);
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn a_normal_map_is_imported() {
    // `scale` keeps its spec default `1.0`: nothing to report.
    let bytes = png();
    let material = r#"{"normalTexture":{"index":0}}"#;
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&bytes)]);

    let pbr = only_pbr(&drawing);
    let key = pbr.normal_texture.expect("the normal map is referenced");
    assert_eq!(texture_bytes(&drawing, &key), bytes.as_slice());
    assert!(
        !codes(&drawing).contains(&DIAG_NORMAL_SCALE_NOT_IMPORTED),
        "a scale of 1.0 is the authored default"
    );
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn a_normal_scale_is_named_and_the_map_is_kept() {
    let bytes = png();
    let material = r#"{"normalTexture":{"index":0,"scale":0.5}}"#;
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&bytes)]);

    let pbr = only_pbr(&drawing);
    assert!(
        pbr.normal_texture.is_some(),
        "the map itself is imported; only the scale is lost"
    );
    assert!(codes(&drawing).contains(&DIAG_NORMAL_SCALE_NOT_IMPORTED));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn an_occlusion_map_keeps_its_strength() {
    let bytes = png();
    let material = r#"{"occlusionTexture":{"index":0,"strength":0.4}}"#;
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&bytes)]);

    let pbr = only_pbr(&drawing);
    assert_close(pbr.occlusion_strength, 0.4);
    let key = pbr
        .occlusion_texture
        .expect("the occlusion map is referenced");
    assert_eq!(texture_bytes(&drawing, &key), bytes.as_slice());
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn an_emissive_factor_and_map_are_imported() {
    let bytes = png();
    let material = r#"{"emissiveFactor":[0.2,0.4,0.6],"emissiveTexture":{"index":0}}"#;
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&bytes)]);

    let pbr = only_pbr(&drawing);
    assert_rgb_close(pbr.emissive, [0.2, 0.4, 0.6]);
    let key = pbr
        .emissive_texture
        .expect("the emissive map is referenced");
    assert_eq!(texture_bytes(&drawing, &key), bytes.as_slice());
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn every_slot_can_share_one_document_texture() {
    // The four channels name the same image: `register_texture` is content
    // addressed, so the document stores one set of bytes.
    let bytes = png();
    let material = r#"{"pbrMetallicRoughness":{"metallicFactor":0.1,
        "metallicRoughnessTexture":{"index":0}},
        "normalTexture":{"index":0},
        "emissiveTexture":{"index":0},
        "emissiveFactor":[0.1,0.1,0.1],
        "occlusionTexture":{"index":0,"strength":0.9}}"#;
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&bytes)]);

    let pbr = only_pbr(&drawing);
    let keys = [
        pbr.metallic_roughness_texture.expect("metallicRoughness"),
        pbr.normal_texture.expect("normal"),
        pbr.emissive_texture.expect("emissive"),
        pbr.occlusion_texture.expect("occlusion"),
    ];
    assert!(
        keys.iter().all(|key| *key == keys[0]),
        "identical bytes must resolve to one content key"
    );
    assert_eq!(drawing.database.textures().count(), 1);
    assert_close(pbr.occlusion_strength, 0.9);
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

// ---------------------------------------------------------------------------
// Named losses.
// ---------------------------------------------------------------------------

#[test]
fn a_pbr_slot_on_another_uv_set_is_dropped_and_noted() {
    let bytes = png();
    let material = r#"{"normalTexture":{"index":0,"texCoord":1}}"#;
    let drawing = import_pbr(material, 2, &[TextureImage::Bin(&bytes)]);

    let pbr = only_pbr(&drawing);
    assert_eq!(
        pbr.normal_texture, None,
        "the domain stores one UV set, so a UV1 slot cannot be sampled"
    );
    assert!(codes(&drawing).contains(&DIAG_UV_SET_IGNORED));
    assert!(
        drawing.database.textures().next().is_none(),
        "no orphan image is registered for an unimported slot"
    );
    // The rest of the material still applies, and an ignored UV1 set is a note.
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn an_out_of_range_factor_drops_the_whole_channel() {
    // glTF does not validate these, and ADR-0039 D3 refuses rather than clamps.
    let bytes = png();
    let material = r#"{"pbrMetallicRoughness":{"metallicFactor":2.0,
        "metallicRoughnessTexture":{"index":0}}}"#;
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&bytes)]);

    assert_eq!(
        meshes(&drawing)[0].pbr,
        None,
        "an unusable factor is not stored as a plausible-looking number"
    );
    assert!(codes(&drawing).contains(&DIAG_PBR_IGNORED));
    assert!(
        drawing.database.textures().next().is_none(),
        "the channel is refused before any texture byte is registered"
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn an_external_pbr_image_is_named_and_the_scalars_stay() {
    let material = r#"{"pbrMetallicRoughness":{"roughnessFactor":0.5,"metallicRoughnessTexture":{"index":0}}}"#;
    let drawing = import_pbr(
        material,
        1,
        &[TextureImage::Entry(r#"{"uri":"mr.png"}"#.to_string())],
    );

    let pbr = only_pbr(&drawing);
    assert_eq!(pbr.metallic_roughness_texture, None);
    assert_close(pbr.roughness, 0.5);
    let codes = codes(&drawing);
    assert!(codes.contains(&DIAG_EXTERNAL_IMAGE));
    assert!(
        !codes.contains(&DIAG_TEXTURE_NOT_IMPORTED),
        "the base-colour code names the base-colour slot only"
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_pbr_texture_without_a_uv_set_is_dropped() {
    let bytes = png();
    let material = r#"{"pbrMetallicRoughness":{"metallicFactor":0.0,
        "metallicRoughnessTexture":{"index":0}}}"#;
    // No `TEXCOORD_0` attribute: the map has nowhere to sample.
    let drawing = import_pbr(material, 0, &[TextureImage::Bin(&bytes)]);

    let mesh = meshes(&drawing)[0];
    assert!(mesh.uvs.is_empty());
    let pbr = only_pbr(&drawing);
    assert_eq!(pbr.metallic_roughness_texture, None);
    assert_close(pbr.metallic, 0.0);
    assert!(codes(&drawing).contains(&DIAG_UVS_DROPPED));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_khr_texture_transform_on_a_pbr_slot_is_reported() {
    let bytes = png();
    let material = r#"{"normalTexture":{"index":0,"extensions":
        {"KHR_texture_transform":{"offset":[0.0,0.0],"scale":[2.0,2.0]}}}}"#;
    let drawing = import_pbr(material, 1, &[TextureImage::Bin(&bytes)]);

    assert!(only_pbr(&drawing).normal_texture.is_some());
    assert!(codes(&drawing).contains(&DIAG_TEXTURE_TRANSFORM_NOT_IMPORTED));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_khr_emissive_strength_extension_is_reported() {
    let material = r#"{"emissiveFactor":[0.0,0.0,0.5],"extensions":
        {"KHR_materials_emissive_strength":{"emissiveStrength":4.0}}}"#;
    let drawing = import_pbr(material, 0, &[]);

    assert_rgb_close(only_pbr(&drawing).emissive, [0.0, 0.0, 0.5]);
    assert!(codes(&drawing).contains(&DIAG_EMISSIVE_STRENGTH_NOT_IMPORTED));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}
