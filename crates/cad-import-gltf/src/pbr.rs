//! The metallic-roughness material channels of one glTF primitive (ADR-0039 D4).
//!
//! Two questions are answered here, and they are deliberately separate:
//!
//! * **may** this primitive have a `MeshPbr` at all? Only when the document JSON
//!   declares one of the channels ([`PbrDecl`]); the `gltf` crate's getters
//!   answer with spec defaults, so they cannot tell "authored 1.0" from "absent".
//! * **which** values and bytes can be stored? The factors must be finite and in
//!   `[0, 1]` (nothing is clamped), and every texture slot samples UV0 because
//!   the domain has exactly one UV set.
//!
//! Every refusal is kept in [`PbrLosses`] until the caller knows the entity id;
//! nothing is dropped silently and no channel is guessed at.

use cad_domain::{CadResult, Diagnostic, MeshPbr, ObjectId};
use cad_import_core::MeshImport;
use gltf::{
    material::{NormalTexture, OcclusionTexture},
    texture::{Info as TextureInfo, Texture},
    Primitive,
};

use crate::buffers::ResolvedBuffer;
use crate::texture::{refusal_marker, resolve_texture_image, PbrDecl, TextureImage};
use crate::{
    DIAG_EMISSIVE_STRENGTH_NOT_IMPORTED, DIAG_EXTERNAL_IMAGE, DIAG_NORMAL_SCALE_NOT_IMPORTED,
    DIAG_PBR_IGNORED, DIAG_TEXTURE_TRANSFORM_NOT_IMPORTED,
};

/// The PBR channel references one primitive's material carries.
///
/// The values come from the `gltf` getters, which answer with spec defaults for
/// keys the document omits; whether a `MeshPbr` may exist at all is decided by
/// [`PbrDecl`] alone, so a `baseColorFactor`-only material never reaches here.
pub(crate) struct PbrChannels<'a> {
    metallic: f32,
    roughness: f32,
    emissive: [f32; 3],
    /// `occlusionTexture.strength`, or the neutral identity `1.0` when the
    /// material has no occlusion map (the value only multiplies that map).
    occlusion_strength: f32,
    metallic_roughness: Option<TextureInfo<'a>>,
    normal: Option<NormalTexture<'a>>,
    emissive_texture: Option<TextureInfo<'a>>,
    occlusion: Option<OcclusionTexture<'a>>,
}

impl<'a> PbrChannels<'a> {
    pub(crate) fn read(primitive: &Primitive<'a>) -> Self {
        let material = primitive.material();
        let pbr = material.pbr_metallic_roughness();
        let occlusion = material.occlusion_texture();
        PbrChannels {
            metallic: pbr.metallic_factor(),
            roughness: pbr.roughness_factor(),
            emissive: material.emissive_factor(),
            occlusion_strength: occlusion.as_ref().map_or(1.0, OcclusionTexture::strength),
            metallic_roughness: pbr.metallic_roughness_texture(),
            normal: material.normal_texture(),
            emissive_texture: material.emissive_texture(),
            occlusion,
        }
    }

    /// The declared factors that are non-finite or outside `[0, 1]`.
    ///
    /// glTF does not validate any of these on parse, and ADR-0039 D3 refuses an
    /// out-of-range value rather than clamping it, so the whole channel is
    /// dropped and named instead of being stored as a plausible-looking number.
    pub(crate) fn invalid_scalars(&self) -> Option<Vec<&'static str>> {
        let mut invalid = Vec::new();
        if !unit_scalar(self.metallic) {
            invalid.push("metallicFactor");
        }
        if !unit_scalar(self.roughness) {
            invalid.push("roughnessFactor");
        }
        if !unit_scalar(self.occlusion_strength) {
            invalid.push("occlusionTexture.strength");
        }
        if self
            .emissive
            .iter()
            .any(|component| !unit_scalar(*component))
        {
            invalid.push("emissiveFactor");
        }
        (!invalid.is_empty()).then_some(invalid)
    }

    /// `normalTexture.scale` when it is authored as something other than `1.0`.
    pub(crate) fn unauthored_normal_scale(&self) -> Option<f32> {
        self.normal
            .as_ref()
            .map(NormalTexture::scale)
            .filter(|scale| *scale != 1.0)
    }

    /// Set 0 when any PBR texture slot samples it, so the caller knows which UV
    /// set to read when there is no base-colour texture to name one.
    pub(crate) fn uv_set(&self) -> Option<u32> {
        self.slot_tex_coords()
            .into_iter()
            .find(|tex_coord| *tex_coord == 0)
    }

    /// Import every declared channel, or name why it cannot be stored.
    ///
    /// Returns the `MeshPbr` (its factors already validated by the caller) and
    /// the `TEXCOORD` sets a slot named but this import could not read, so the
    /// document note can report them. The channels share UV0, so a slot on any
    /// other set is not imported; a slot whose UV set was unusable stays unset
    /// while the scalar factors still apply.
    pub(crate) fn import(
        &self,
        uv_set: Option<u32>,
        uvs_usable: bool,
        buffers: &[ResolvedBuffer<'_>],
        import: &mut MeshImport,
        losses: &mut PbrLosses,
    ) -> CadResult<Option<MeshPbr>> {
        let mut declared = MeshPbr {
            metallic: f64::from(self.metallic),
            roughness: f64::from(self.roughness),
            emissive: self.emissive.map(f64::from),
            occlusion_strength: f64::from(self.occlusion_strength),
            ..Default::default()
        };
        let mut ignored_uv_sets = Vec::new();
        let slots: [(&'static str, Option<(u32, Texture<'_>)>); 4] = [
            (
                "metallicRoughness",
                self.metallic_roughness
                    .as_ref()
                    .map(|info| (info.tex_coord(), info.texture())),
            ),
            (
                "normal",
                self.normal
                    .as_ref()
                    .map(|normal| (normal.tex_coord(), normal.texture())),
            ),
            (
                "emissive",
                self.emissive_texture
                    .as_ref()
                    .map(|info| (info.tex_coord(), info.texture())),
            ),
            (
                "occlusion",
                self.occlusion
                    .as_ref()
                    .map(|occlusion| (occlusion.tex_coord(), occlusion.texture())),
            ),
        ];
        for (slot, reference) in slots {
            let key = import_slot(
                slot,
                reference,
                uv_set,
                uvs_usable,
                buffers,
                import,
                losses,
                &mut ignored_uv_sets,
            )?;
            match slot {
                "metallicRoughness" => declared.metallic_roughness_texture = key,
                "normal" => declared.normal_texture = key,
                "emissive" => declared.emissive_texture = key,
                "occlusion" => declared.occlusion_texture = key,
                // The array above only contains these four names.
                _ => unreachable!("the PBR slot list is closed"),
            }
        }
        losses.ignored_uv_sets = ignored_uv_sets;
        Ok(Some(declared))
    }

    fn slot_tex_coords(&self) -> Vec<u32> {
        let mut slots = Vec::new();
        if let Some(info) = &self.metallic_roughness {
            slots.push(info.tex_coord());
        }
        if let Some(normal) = &self.normal {
            slots.push(normal.tex_coord());
        }
        if let Some(info) = &self.emissive_texture {
            slots.push(info.tex_coord());
        }
        if let Some(occlusion) = &self.occlusion {
            slots.push(occlusion.tex_coord());
        }
        slots
    }
}

/// Import one PBR texture slot against UV0, or name the loss.
#[allow(clippy::too_many_arguments)]
fn import_slot(
    slot: &'static str,
    reference: Option<(u32, Texture<'_>)>,
    uv_set: Option<u32>,
    uvs_usable: bool,
    buffers: &[ResolvedBuffer<'_>],
    import: &mut MeshImport,
    losses: &mut PbrLosses,
    ignored_uv_sets: &mut Vec<u32>,
) -> CadResult<Option<String>> {
    let Some((tex_coord, texture)) = reference else {
        return Ok(None);
    };
    if uv_set != Some(tex_coord) {
        ignored_uv_sets.push(tex_coord);
        return Ok(None);
    }
    if !uvs_usable {
        // `uvs_dropped` already names this loss, and the scalar factors still
        // apply; a texture with no usable parameterisation could never be drawn.
        return Ok(None);
    }
    match resolve_texture_image(&texture, buffers)? {
        TextureImage::Embedded(bytes) => match import.register_texture(&bytes) {
            Ok(key) => Ok(Some(key)),
            Err(error) => {
                // The refusal's own stable marker (`import.texture.too_large` and
                // friends) is kept, and only this slot is lost.
                let message = error.to_string();
                losses
                    .refused
                    .push((slot, refusal_marker(&message).to_string(), message));
                Ok(None)
            }
        },
        TextureImage::Unavailable { uri } => {
            losses.external.push((slot, uri));
            Ok(None)
        }
    }
}

/// Every PBR loss one primitive incurred, reported once the object id is known.
#[derive(Default)]
pub(crate) struct PbrLosses {
    /// Declared PBR factors that are non-finite or outside `[0, 1]`.
    ignored: Option<Vec<&'static str>>,
    /// The material carried `KHR_materials_emissive_strength`.
    emissive_strength: bool,
    /// PBR slots whose `textureInfo` carried `KHR_texture_transform`.
    transformed_slots: Vec<&'static str>,
    /// `normalTexture.scale` as authored, when it is not `1.0`.
    normal_scale: Option<f32>,
    /// PBR slots whose image names bytes this bytes-only import cannot read.
    external: Vec<(&'static str, String)>,
    /// PBR slots `register_texture` refused: slot, stable marker, message.
    refused: Vec<(&'static str, String, String)>,
    /// `TEXCOORD` sets a PBR slot named but this import did not read.
    pub(crate) ignored_uv_sets: Vec<u32>,
}

impl PbrLosses {
    /// The document-level facts about the material that no channel can carry.
    pub(crate) fn declared(declaration: &PbrDecl) -> Self {
        PbrLosses {
            emissive_strength: declaration.emissive_strength,
            transformed_slots: declaration.transformed_slots.clone(),
            ..Default::default()
        }
    }

    /// Record factors that cannot be stored; the channel is dropped whole.
    pub(crate) fn ignore(&mut self, invalid: Vec<&'static str>) {
        self.ignored = Some(invalid);
    }

    /// Whether a declared factor was unusable, so no channel may be imported.
    pub(crate) fn is_ignored(&self) -> bool {
        self.ignored.is_some()
    }

    /// Record a `normalTexture.scale` that the domain cannot store.
    pub(crate) fn set_normal_scale(&mut self, scale: Option<f32>) {
        self.normal_scale = scale;
    }

    /// Report every loss, in a fixed order, against `object`.
    pub(crate) fn record(&self, import: &mut MeshImport, object: Option<ObjectId>, label: &str) {
        if let Some(invalid) = &self.ignored {
            import.record(
                Diagnostic {
                    object,
                    code: DIAG_PBR_IGNORED.into(),
                    message: format!(
                        "{label}: the material's {} is non-finite or outside [0, 1]; no PBR \
                         channel was imported for this primitive (nothing is clamped)",
                        invalid.join(", ")
                    ),
                },
                Some(format!(
                    "{label}: a declared PBR factor was unusable and the PBR channels were dropped"
                )),
            );
        }
        for (slot, code, message) in &self.refused {
            import.record(
                Diagnostic {
                    object,
                    code: code.clone(),
                    message: format!("{label}: the {slot} texture was refused: {message}"),
                },
                Some(format!("{label}: a PBR {slot} texture was refused")),
            );
        }
        for (slot, uri) in &self.external {
            import.record(
                Diagnostic {
                    object,
                    code: DIAG_EXTERNAL_IMAGE.into(),
                    message: format!(
                        "{label}: the {slot} image is `{uri}`, which this bytes-only import cannot \
                         read; that PBR channel keeps its scalar factor only"
                    ),
                },
                Some(format!("{label}: a PBR {slot} image is an external file")),
            );
        }
        if !self.transformed_slots.is_empty() {
            let slots = self.transformed_slots.join(", ");
            import.record(
                Diagnostic {
                    object,
                    code: DIAG_TEXTURE_TRANSFORM_NOT_IMPORTED.into(),
                    message: format!(
                        "{label}: the {slots} texture info carries KHR_texture_transform; the \
                         transform is not applied and the UVs are stored as authored"
                    ),
                },
                Some(format!(
                    "{label}: a PBR KHR_texture_transform was not applied"
                )),
            );
        }
        if let Some(scale) = self.normal_scale {
            import.record(
                Diagnostic {
                    object,
                    code: DIAG_NORMAL_SCALE_NOT_IMPORTED.into(),
                    message: format!(
                        "{label}: normalTexture.scale is {scale}; the scale is not applied and the \
                         normal map is stored as authored"
                    ),
                },
                Some(format!(
                    "{label}: a normalTexture.scale other than 1.0 was not applied"
                )),
            );
        }
        if self.emissive_strength {
            import.record(
                Diagnostic {
                    object,
                    code: DIAG_EMISSIVE_STRENGTH_NOT_IMPORTED.into(),
                    message: format!(
                        "{label}: the material carries KHR_materials_emissive_strength; the \
                         emissive multiplier is not applied"
                    ),
                },
                Some(format!(
                    "{label}: a KHR_materials_emissive_strength multiplier was not applied"
                )),
            );
        }
    }
}

/// A glTF scalar the domain stores in `[0, 1]`.
fn unit_scalar(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
