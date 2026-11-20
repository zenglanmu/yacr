//! glTF PBR scalars for the RVT preview lane (ADR-0039 D4).
//!
//! `rvt` 0.24.0 writes `pbrMetallicRoughness.metallicFactor`/`roughnessFactor`
//! for every material it emits, but its own `src/ifc/pbr.rs` derives both
//! **heuristically from the material name**; `src/ifc/gltf.rs` says as much
//! ("Metallic and roughness are always rvt-rs's"). They are therefore imported
//! as values that approximate a surface, and the importer reports
//! `import.rvt.pbr_heuristic` once per document so neither the document nor the
//! renderer presents them as Revit-authored material data.
//!
//! This module owns the two questions the GLB reader has to answer before it
//! can build a [`cad_domain::MeshPbr`]:
//!
//! 1. **Does the material declare either scalar?** The typed `gltf` view cannot
//!    tell, because `gltf-json` fills in the spec default (`1.0`) for an absent
//!    key ([`declares_pbr_factor`] reads the raw chunk instead).
//! 2. **Is the pair usable?** Both must be finite and inside `[0, 1]`
//!    ([`validate_pbr_factors`]); otherwise no `MeshPbr` is stored at all and
//!    the loss is counted — never clamped (ADR-0039 D4, invariant 2).

/// The GLB's JSON chunk, re-read from the source bytes.
///
/// `gltf` 1.4.1 deserialises `pbrMetallicRoughness` into a typed struct whose
/// `metallicFactor`/`roughnessFactor` are `#[serde(default)]` (spec default
/// `1.0`), so the typed view cannot tell "declared `1.0`" from "not declared".
/// ADR-0039 D1 requires a material that declares no PBR scalar to stay
/// `Mesh.pbr = None` (glTF's default metallic `1.0` would draw an unlit CAD
/// body as black metal), so the presence check reads the raw JSON chunk.
///
/// `gltf` already parsed this same chunk, so a parse failure here is unreachable
/// in practice; `None` then means "no PBR scalar is imported", never a
/// fabricated spec default.
pub(crate) fn json_chunk(bytes: &[u8]) -> Option<serde_json::Value> {
    // GLB: 12-byte header (magic, version, length), then the JSON chunk's
    // 4-byte length and 4-byte type, then its bytes.
    let length = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize;
    let end = 20usize.checked_add(length)?;
    serde_json::from_slice(bytes.get(20..end)?).ok()
}

/// Whether a material's raw JSON declares `metallicFactor` or `roughnessFactor`.
///
/// The rule (ADR-0039 D1/D4): a `MeshPbr` is created only when the material
/// **declares** at least one of the two scalars. If only one is declared, the
/// other keeps the glTF spec default the writer meant, which is a spec value
/// rather than an invented one. A material that declares neither — the
/// `baseColorFactor`-only case, i.e. any writer that does not emit PBR — stays
/// `Mesh.pbr = None`: glTF's default `metallicFactor = 1.0` would turn an unlit
/// CAD body into black metal.
///
/// `rvt` 0.24.0 always writes both, so "declares neither" appears only on
/// foreign or synthetic input; the reader states the rule instead of relying on
/// that.
pub(crate) fn declares_pbr_factor(raw: Option<&serde_json::Value>, index: usize) -> bool {
    raw.and_then(|document| document.get("materials"))
        .and_then(|materials| materials.get(index))
        .and_then(|material| material.get("pbrMetallicRoughness"))
        .is_some_and(|pbr| {
            pbr.get("metallicFactor").is_some() || pbr.get("roughnessFactor").is_some()
        })
}

/// Validate a glTF `metallicFactor`/`roughnessFactor` pair.
///
/// Both must be finite and inside glTF's `[0, 1]` range. A violating pair yields
/// `None` — the whole PBR channel is dropped, never clamped and never half
/// applied: a metallic of `2.0` beside a roughness of `0.5` says nothing
/// trustworthy about the material (ADR-0039 D4, invariant 2). `rvt`'s
/// name-based heuristics stay inside the range, so this fires only on foreign or
/// corrupt input.
pub(crate) fn validate_pbr_factors(metallic: f32, roughness: f32) -> Option<(f32, f32)> {
    let finite = metallic.is_finite() && roughness.is_finite();
    let in_range = (0.0..=1.0).contains(&metallic) && (0.0..=1.0).contains(&roughness);
    (finite && in_range).then_some((metallic, roughness))
}
