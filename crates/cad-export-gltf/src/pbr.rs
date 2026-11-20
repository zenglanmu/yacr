//! PBR channel emission for the glTF writer (ADR-0039 D5).
//!
//! The source is `Mesh::pbr`, which the importer only fills when the document
//! declared a channel. Everything here is therefore "declared PBR": there is no
//! path from `pbr: None` to a written PBR key, which is what keeps an untextured,
//! un-PBR export byte-identical to the pre-ADR-0039 output.
//!
//! Two halves live here and they share one rule — a channel is written exactly
//! when its image bytes and its UV0 are both writable, and otherwise the loss is
//! one `export.gltf.texture_not_written` conversion:
//!
//! * [`pbr_textures`] registers every image and returns the `textures[]` indices;
//! * [`PbrKey`] extends the material dedup key and [`material_fields`] writes the
//!   material's PBR properties, so two meshes with different PBR values never
//!   share a material.

use cad_domain::ObjectId;
use serde_json::{json, Value};

use crate::collect::{PbrChannel, SourcePbr};
use crate::report::ReportBuilder;
use crate::texture::{texture_mime, Textures, Uv0};
use crate::write::{Builder, TEXTURE_NOT_WRITTEN};

/// The `textures[]` indices one mesh's PBR channels reach.
#[derive(Default)]
pub(crate) struct PbrTextures {
    pub(crate) metallic_roughness: Option<usize>,
    pub(crate) normal: Option<usize>,
    pub(crate) emissive: Option<usize>,
    pub(crate) occlusion: Option<usize>,
}

impl PbrTextures {
    /// Whether any channel reached an image, so the primitive needs `TEXCOORD_0`.
    pub(crate) fn any(&self) -> bool {
        self.metallic_roughness.is_some()
            || self.normal.is_some()
            || self.emissive.is_some()
            || self.occlusion.is_some()
    }
}

/// Register every PBR image the mesh declares and report the ones that cannot be
/// written.
///
/// The second element is `true` when at least one channel was lost, which keeps
/// the mesh out of the `exact` count. `None` means the mesh declared no PBR at
/// all — not "a PBR material with no textures" — so a `pbr: None` export writes
/// no PBR key and no extra image.
pub(crate) fn pbr_textures(
    object: ObjectId,
    pbr: &SourcePbr,
    uv0: &Uv0,
    textures: &mut Textures,
    builder: &mut Builder,
    report: &mut ReportBuilder,
) -> (PbrTextures, bool) {
    let mut indices = PbrTextures::default();
    let mut lost = false;
    indices.metallic_roughness = channel_index(
        &pbr.metallic_roughness,
        "metallicRoughness",
        object,
        uv0,
        textures,
        builder,
        report,
        &mut lost,
    );
    indices.normal = channel_index(
        &pbr.normal,
        "normal",
        object,
        uv0,
        textures,
        builder,
        report,
        &mut lost,
    );
    indices.emissive = channel_index(
        &pbr.emissive_texture,
        "emissive",
        object,
        uv0,
        textures,
        builder,
        report,
        &mut lost,
    );
    indices.occlusion = channel_index(
        &pbr.occlusion,
        "occlusion",
        object,
        uv0,
        textures,
        builder,
        report,
        &mut lost,
    );
    (indices, lost)
}

/// Register one PBR channel's image and return its `textures[]` index.
#[allow(clippy::too_many_arguments)]
fn channel_index(
    channel: &PbrChannel,
    channel_name: &str,
    object: ObjectId,
    uv0: &Uv0,
    textures: &mut Textures,
    builder: &mut Builder,
    report: &mut ReportBuilder,
    lost: &mut bool,
) -> Option<usize> {
    let texture = match channel {
        PbrChannel::None => return None,
        PbrChannel::Missing(key) => {
            *lost = true;
            report.converted(
                TEXTURE_NOT_WRITTEN,
                format!(
                    "entity #{} names a PBR {channel_name} texture {key:?} but the document does \
                     not carry its bytes; that channel was not written",
                    object.0
                ),
            );
            return None;
        }
        PbrChannel::Bytes(texture) => texture,
    };
    let Some(mime) = texture_mime(&texture.key, &texture.bytes) else {
        *lost = true;
        report.converted(
            TEXTURE_NOT_WRITTEN,
            format!(
                "entity #{} PBR {channel_name} texture {:?} does not carry bytes matching its \
                 extension; that channel was not written",
                object.0, texture.key
            ),
        );
        return None;
    };
    if uv0.pairs().is_none() {
        // Every PBR channel samples UV0; an image with no parameterisation would
        // be written but never drawn.
        *lost = true;
        report.converted(
            TEXTURE_NOT_WRITTEN,
            format!(
                "entity #{} names a PBR {channel_name} texture {:?} but has no writable UV0; that \
                 channel was not written",
                object.0, texture.key
            ),
        );
        return None;
    }
    Some(textures.index_for(builder, &texture.key, &texture.bytes, mime))
}

/// The part of a material's identity that comes from its PBR channels.
///
/// Every stored value joins the key as its own bit pattern, plus the four
/// resolved `textures[]` indices: two meshes that differ in a single scalar or
/// name a different image get two materials, so a written material never claims
/// a channel it does not have.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PbrKey {
    metallic_bits: u64,
    roughness_bits: u64,
    emissive_bits: [u64; 3],
    occlusion_strength_bits: u64,
    metallic_roughness: Option<usize>,
    normal: Option<usize>,
    emissive_texture: Option<usize>,
    occlusion: Option<usize>,
}

impl PbrKey {
    pub(crate) fn new(pbr: &SourcePbr, textures: &PbrTextures) -> Self {
        PbrKey {
            metallic_bits: pbr.metallic.to_bits(),
            roughness_bits: pbr.roughness.to_bits(),
            emissive_bits: pbr.emissive.map(f64::to_bits),
            occlusion_strength_bits: pbr.occlusion_strength.to_bits(),
            metallic_roughness: textures.metallic_roughness,
            normal: textures.normal,
            emissive_texture: textures.emissive,
            occlusion: textures.occlusion,
        }
    }
}

/// The PBR properties of one `pbrMetallicRoughness` object (ADR-0039 D5).
///
/// `metallicFactor`/`roughnessFactor` are always written — they are the two
/// scalars glTF multiplies its channels by — while `metallicRoughnessTexture`
/// only exists when that image could be written.
pub(crate) fn metallic_roughness_fields(
    pbr: &SourcePbr,
    textures: &PbrTextures,
) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("metallicFactor", Value::from(pbr.metallic)),
        ("roughnessFactor", Value::from(pbr.roughness)),
    ];
    if let Some(index) = textures.metallic_roughness {
        fields.push((
            "metallicRoughnessTexture",
            json!({ "index": index, "texCoord": 0 }),
        ));
    }
    fields
}

/// The PBR properties that sit on the material itself (ADR-0039 D5).
///
/// `normalTexture.scale` is the authored `1.0`: the domain stores no separate
/// scale, and a source scale other than `1.0` was reported at import time.
/// `occlusionTexture.strength` is the stored strength, and `emissiveFactor` is
/// written whenever the channels are, so a re-import sees a declared PBR
/// material even when every value is its neutral.
pub(crate) fn material_fields(
    pbr: &SourcePbr,
    textures: &PbrTextures,
) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    if let Some(index) = textures.normal {
        fields.push((
            "normalTexture",
            json!({ "index": index, "texCoord": 0, "scale": 1.0 }),
        ));
    }
    if let Some(index) = textures.occlusion {
        fields.push((
            "occlusionTexture",
            json!({ "index": index, "texCoord": 0, "strength": pbr.occlusion_strength }),
        ));
    }
    if let Some(index) = textures.emissive {
        fields.push(("emissiveTexture", json!({ "index": index, "texCoord": 0 })));
    }
    fields.push((
        "emissiveFactor",
        json!([pbr.emissive[0], pbr.emissive[1], pbr.emissive[2]]),
    ));
    fields
}
