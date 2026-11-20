//! PBR channel-set types (ADR-0039 D6).
//!
//! Split out of `lib.rs` to keep the crate root within the file-size gate; the
//! renderer's `pbr_bind_groups` cache and the all-identity fallback bind group
//! are keyed on these two types.

use super::*;

/// The five texture keys a PBR batch binds at group 2 (ADR-0039 D6).
///
/// `base` is the batch's existing base-colour key; the other four are the PBR
/// slots. Deriving `Ord` makes the bind-group cache a deterministic
/// [`std::collections::BTreeMap`] and keeps a diagnostic listing stable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PbrTextureSet {
    pub(crate) base: Option<ResourceKey>,
    pub(crate) metallic_roughness: Option<ResourceKey>,
    pub(crate) normal: Option<ResourceKey>,
    pub(crate) emissive: Option<ResourceKey>,
    pub(crate) occlusion: Option<ResourceKey>,
}

impl PbrTextureSet {
    /// The channel set a resident batch draws with.
    pub(crate) fn of(batch: &GpuBatch) -> Self {
        PbrTextureSet {
            base: batch.texture.clone(),
            metallic_roughness: batch
                .pbr
                .as_ref()
                .and_then(|p| p.metallic_roughness.clone()),
            normal: batch.pbr.as_ref().and_then(|p| p.normal.clone()),
            emissive: batch.pbr.as_ref().and_then(|p| p.emissive_texture.clone()),
            occlusion: batch.pbr.as_ref().and_then(|p| p.occlusion.clone()),
        }
    }
}

/// The exact multiplicative-identity 1x1 textures bound for a disabled PBR
/// channel (ADR-0039 D6).
///
/// * `white` — base colour, metallic-roughness (`g` = 1, `b` = 1) and occlusion
///   (`r` = 1) all sample as a neutral 1, so a disabled channel multiplies by 1;
/// * `black` — emissive adds nothing;
/// * `flat_normal` — `(0.5, 0.5, 1)` decodes to the tangent-space `(0, 0, 1)`,
///   i.e. "no perturbation".
///
/// This is the standard "channel disabled" identity, **not** fabricated material
/// data: the report still records that the source declared no such channel.
pub(crate) struct PbrIdentities {
    pub(crate) white: wgpu::TextureView,
    pub(crate) black: wgpu::TextureView,
    pub(crate) flat_normal: wgpu::TextureView,
    /// Kept alive for the views above (a view does not own its texture).
    pub(crate) _textures: [wgpu::Texture; 3],
}
