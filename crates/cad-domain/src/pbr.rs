//! A mesh's declared PBR channels (ADR-0039 D1).
//!
//! [`crate::Mesh::pbr`] is `Option<MeshPbr>` and the `None` case is meaningful:
//! it means **the source declared no PBR material at all**, not "the default
//! material". A glTF material that only carries `baseColorFactor`/
//! `baseColorTexture` stays `None` and keeps rendering through the existing
//! unlit/textured channels. Filling in glTF's spec defaults (`metallicFactor =
//! 1`) would draw an unlit CAD asset as black metal, which is fabricated data,
//! not fidelity.
//!
//! The scalars are stored as `f64` to match [`crate::Mesh`]'s other components
//! and the ymodel numeric codec; the renderer converts to `f32`. `cad-db`
//! enforces the ranges below, so the domain stays a plain data view:
//!
//! * `metallic`, `roughness` and `occlusion_strength` are in `[0, 1]`;
//! * `emissive` is **linear** RGB in `[0, 1]` (not sRGB — the shader adds it
//!   after decoding every texture);
//! * a texture key is a **document texture key** in the ADR-0038 form
//!   (`<32 lowercase hex>.png|.jpg`, see [`crate::is_valid_texture_key`]); the
//!   bytes live in `cad-db` and in the archive's `textures/<key>` entries, so
//!   nothing here is a platform path.
//!
//! Every texture channel samples **UV0** — the domain has exactly one UV set
//! (ADR-0038 D1) — which is why `cad-db` refuses any PBR texture on a mesh
//! without UVs.

/// The metallic-roughness material channels a mesh declared (ADR-0039 D1).
///
/// Defaults are all zero/absent because a `MeshPbr` only exists when the source
/// declared PBR; nothing here is a spec default that gets filled in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshPbr {
    /// Metallic factor in `[0, 1]`.
    pub metallic: f64,
    /// Roughness factor in `[0, 1]`.
    pub roughness: f64,
    /// **Linear** RGB emissive factor, each component in `[0, 1]`.
    pub emissive: [f64; 3],
    /// Ambient-occlusion strength in `[0, 1]`; only meaningful with
    /// [`Self::occlusion_texture`].
    pub occlusion_strength: f64,
    /// Document texture key of the metallic-roughness map (`g` = roughness,
    /// `b` = metallic), multiplied by the two scalars.
    pub metallic_roughness_texture: Option<String>,
    /// Document texture key of the tangent-space normal map (UV0).
    pub normal_texture: Option<String>,
    /// Document texture key of the emissive map (UV0, sRGB-decoded).
    pub emissive_texture: Option<String>,
    /// Document texture key of the ambient-occlusion map (UV0).
    pub occlusion_texture: Option<String>,
}
