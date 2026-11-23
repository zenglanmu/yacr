//! Texture-image resolution and material-declaration scanning for the glTF
//! importer (ADR-0038 D6, ADR-0039 D4).
//!
//! A bytes-only import can read exactly two image sources: bytes the document
//! itself carries (a `bufferView` into a resolved buffer) and an embedded base64
//! `data:` URI. An external file is named and reported, never fetched and never
//! replaced with a placeholder. The base-colour slot (ADR-0038) and the four
//! PBR slots (ADR-0039) all resolve their bytes here, so one document image is
//! read one way and registered under one content key no matter how many
//! channels name it.
//!
//! Two facts about a material only the **document JSON** states are read from
//! it as well, from the same single parse the version gate uses:
//!
//! * `KHR_texture_transform` on the base-colour slot (the `gltf` crate here is
//!   built without that extension's typed accessors);
//! * which PBR channels the author actually **declared**. The crate's getters
//!   answer with spec defaults (`metallicFactor = 1`, `roughnessFactor = 1`,
//!   `emissiveFactor = [0, 0, 0]`), which cannot tell "the author wrote 1.0"
//!   from "the author wrote nothing" — and ADR-0039 D1 makes that distinction
//!   load-bearing.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use cad_domain::{CadError, CadResult, Diagnostic, ObjectId};
use cad_connector_core::MeshImport;
use gltf::{
    image::Source as ImageSource,
    texture::{Info as TextureInfo, Texture},
};

use crate::buffers::{data_uri_payload, ResolvedBuffer};
use crate::{DIAG_IMAGE_BASE64, DIAG_IMAGE_OUT_OF_RANGE, DIAG_SAMPLER_WRAP_NOT_IMPORTED};

/// The bytes of one texture image, or why they are not part of the import.
pub(crate) enum TextureImage<'a> {
    /// Encoded PNG/JPEG bytes carried by the document itself.
    Embedded(Cow<'a, [u8]>),
    /// The image names an external file (or a non-base64 data URI).
    Unavailable { uri: String },
}

/// Resolve a `textureInfo` slot's encoded bytes without touching the filesystem.
pub(crate) fn resolve_texture_info<'a>(
    info: &TextureInfo<'a>,
    buffers: &'a [ResolvedBuffer<'a>],
) -> CadResult<TextureImage<'a>> {
    resolve_texture_image(&info.texture(), buffers)
}

/// Resolve a texture's encoded bytes without touching the filesystem.
///
/// A `bufferView` image is sliced out of its already-resolved buffer; a URI image
/// is decoded only when it is an embedded base64 `data:` URI. Anything else is an
/// external file this bytes-only import cannot read, which the caller reports
/// instead of guessing at. Used for the base-colour slot and for every PBR slot,
/// so `normalTexture`/`occlusionTexture` — which the `gltf` crate hands out as
/// `Texture`s rather than `textureInfo`s — resolve exactly like `baseColorTexture`.
///
/// `gltf::Gltf::from_slice` validates the document before this runs, so the
/// `unwrap`s inside `Image::source()` (a `bufferView` image must name an existing
/// view and a MIME type) are only reachable for a document that bypassed
/// validation. This reader relies on that gate rather than catching the panic: an
/// invalid document is a parse failure, not a loss to report.
pub(crate) fn resolve_texture_image<'a>(
    texture: &Texture<'a>,
    buffers: &'a [ResolvedBuffer<'a>],
) -> CadResult<TextureImage<'a>> {
    let image = texture.source();
    match image.source() {
        // The declared MIME type is not trusted: `register_texture` sniffs the
        // container, so a `.png` key can never name JPEG bytes.
        ImageSource::View { view, .. } => {
            let start = view.offset();
            let end = start.checked_add(view.length()).ok_or_else(|| {
                CadError::CorruptData(format!(
                    "{DIAG_IMAGE_OUT_OF_RANGE}: a texture image's buffer view has a length that \
                     overflows a usize"
                ))
            })?;
            let buffer = buffers
                .get(view.buffer().index())
                .ok_or_else(|| {
                    CadError::CorruptData(format!(
                        "{DIAG_IMAGE_OUT_OF_RANGE}: a texture image's buffer view names buffer {}, \
                         which the document does not declare",
                        view.buffer().index()
                    ))
                })?
                .as_slice();
            let bytes = buffer.get(start..end).ok_or_else(|| {
                CadError::CorruptData(format!(
                    "{DIAG_IMAGE_OUT_OF_RANGE}: a texture image's buffer view {start}..{end} is \
                     outside a {}-byte buffer",
                    buffer.len()
                ))
            })?;
            Ok(TextureImage::Embedded(Cow::Borrowed(bytes)))
        }
        ImageSource::Uri { uri, .. } => match data_uri_payload(uri) {
            Some(payload) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(payload)
                    .map_err(|error| {
                        CadError::CorruptData(format!(
                            "{DIAG_IMAGE_BASE64}: a texture image's data URI is not valid base64: \
                             {error}"
                        ))
                    })?;
                Ok(TextureImage::Embedded(Cow::Owned(bytes)))
            }
            // An external file, or a data URI that is not base64: both name bytes
            // this import cannot read.
            None => Ok(TextureImage::Unavailable {
                uri: abbreviated_uri(uri),
            }),
        },
    }
}

/// The stable marker a `register_texture` refusal starts with
/// (`import.texture.too_large` and friends), so the diagnostic keeps that reason.
pub(crate) fn refusal_marker(message: &str) -> &str {
    message
        .split_once(':')
        .map_or(message, |(marker, _)| marker)
        .trim()
}

/// The leading part of a URI, for a diagnostic.
///
/// `data:` payloads can be megabytes long, so a message must not carry one whole.
fn abbreviated_uri(uri: &str) -> String {
    const MAX: usize = 64;
    if uri.len() <= MAX {
        return uri.to_string();
    }
    let mut end = MAX;
    while !uri.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &uri[..end])
}

/// glTF 2.0's default `wrapS`/`wrapT`: `REPEAT` (`10497`).
const GLTF_REPEAT: u64 = 10497;

/// A sampler wrap a texture declares that is not glTF's `REPEAT` default.
///
/// The raw numeric values are kept so the diagnostic can state exactly what the
/// document authored instead of mapping an unknown value onto a spec enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WrapDeclaration {
    pub(crate) wrap_s: u64,
    pub(crate) wrap_t: u64,
}

/// The non-`REPEAT` sampler wrap each texture declares, keyed by texture index.
///
/// A texture with no `sampler`, a sampler of `{}`, or explicit `10497` is glTF's
/// REPEAT default and is **absent** from the map: the renderer samples REPEAT, so
/// those declarations are already honoured. Any other value is exactly what this
/// import cannot carry (`Mesh`/`MeshPbr` have no wrap field and `.ymodel` is not
/// extended), so the caller names it rather than reinterpreting it.
pub(crate) fn wrap_declarations(document: &serde_json::Value) -> BTreeMap<usize, WrapDeclaration> {
    let mut declarations = BTreeMap::new();
    let Some(textures) = document
        .get("textures")
        .and_then(serde_json::Value::as_array)
    else {
        return declarations;
    };
    let samplers = document
        .get("samplers")
        .and_then(serde_json::Value::as_array);
    for (index, texture) in textures.iter().enumerate() {
        // `Gltf::from_slice` validates the document before this runs, so a named
        // sampler index is inside `samplers` and a wrap value is an integer.
        let Some(sampler_index) = texture
            .get("sampler")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
        else {
            continue;
        };
        let Some(sampler) = samplers.and_then(|samplers| samplers.get(sampler_index)) else {
            continue;
        };
        let wrap_s = sampler
            .get("wrapS")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(GLTF_REPEAT);
        let wrap_t = sampler
            .get("wrapT")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(GLTF_REPEAT);
        if wrap_s != GLTF_REPEAT || wrap_t != GLTF_REPEAT {
            declarations.insert(index, WrapDeclaration { wrap_s, wrap_t });
        }
    }
    declarations
}

/// One distinct non-REPEAT wrap declaration and the slots that named it.
struct WrapLoss {
    declaration: WrapDeclaration,
    slots: Vec<&'static str>,
}

/// Every non-REPEAT sampler wrap a primitive's imported textures declare.
///
/// Reported once per **distinct** `(wrapS, wrapT)` pair, not once per slot: an
/// asset that puts one sampler on all five material slots produces one
/// diagnostic naming all of them. The renderer samples REPEAT, so any other
/// declaration is a named loss, never a silent reinterpretation.
#[derive(Default)]
pub(crate) struct WrapLosses {
    losses: Vec<WrapLoss>,
}

impl WrapLosses {
    /// Record the wrap `texture` declares, if it is not REPEAT.
    ///
    /// Called only when the slot is actually imported (its bytes were read and
    /// registered): a slot whose image is external or refused carries no
    /// sampler the renderer could honour anyway.
    pub(crate) fn record(
        &mut self,
        slot: &'static str,
        texture: &Texture<'_>,
        declarations: &BTreeMap<usize, WrapDeclaration>,
    ) {
        let Some(declaration) = declarations.get(&texture.index()) else {
            return;
        };
        match self
            .losses
            .iter_mut()
            .find(|loss| loss.declaration == *declaration)
        {
            Some(loss) => {
                if !loss.slots.contains(&slot) {
                    loss.slots.push(slot);
                }
            }
            None => self.losses.push(WrapLoss {
                declaration: *declaration,
                slots: vec![slot],
            }),
        }
    }

    /// Report every distinct declaration against `object`.
    pub(crate) fn record_diagnostics(
        &self,
        import: &mut MeshImport,
        object: Option<ObjectId>,
        label: &str,
    ) {
        for loss in &self.losses {
            let slots = loss.slots.join(", ");
            import.record(
                Diagnostic {
                    object,
                    code: DIAG_SAMPLER_WRAP_NOT_IMPORTED.into(),
                    message: format!(
                        "{label}: the {slots} texture declares sampler wrap (wrapS {}, wrapT \
                         {}); this import does not carry sampler wrap and the renderer samples \
                         mesh textures with REPEAT",
                        loss.declaration.wrap_s, loss.declaration.wrap_t
                    ),
                },
                Some(format!(
                    "{label}: a texture sampler wrap other than REPEAT was not applied (mesh \
                     textures are sampled with REPEAT)"
                )),
            );
        }
    }
}

/// The materials whose **base-colour** texture info carries
/// `KHR_texture_transform`.
///
/// The `gltf` crate here is built without its `KHR_texture_transform` and
/// `extensions` features, so `texture::Info::texture_transform()` does not exist.
/// The document JSON is inspected instead — the same parse the version check
/// needs anyway, so this costs no extra parse. Only the base-colour slot counts
/// here: a transform on a PBR slot is that channel's own loss, reported through
/// [`PbrDecl::transformed_slots`].
pub(crate) fn transform_materials(document: &serde_json::Value) -> BTreeSet<usize> {
    let mut materials = BTreeSet::new();
    let Some(entries) = document
        .get("materials")
        .and_then(serde_json::Value::as_array)
    else {
        return materials;
    };
    for (index, material) in entries.iter().enumerate() {
        let transformed = material
            .get("pbrMetallicRoughness")
            .and_then(|pbr| pbr.get("baseColorTexture"))
            .and_then(|info| info.get("extensions"))
            .and_then(|extensions| extensions.get("KHR_texture_transform"))
            .is_some();
        if transformed {
            materials.insert(index);
        }
    }
    materials
}

/// What one material's JSON declares about PBR (ADR-0039 D4).
///
/// Nothing here is a spec default: `declared` is `true` only when the document
/// spells out at least one PBR scalar/texture, so a material that carries only
/// `baseColorFactor`/`baseColorTexture` stays `pbr: None` in the domain.
#[derive(Default)]
pub(crate) struct PbrDecl {
    /// The document declares at least one PBR scalar or texture for this
    /// material (`metallicFactor`, `roughnessFactor`, `metallicRoughnessTexture`,
    /// `normalTexture`, `occlusionTexture`, `emissiveTexture`, or a non-zero
    /// `emissiveFactor`).
    pub(crate) declared: bool,
    /// The material carries `KHR_materials_emissive_strength` (any value).
    pub(crate) emissive_strength: bool,
    /// PBR texture slots whose `textureInfo` carries `KHR_texture_transform`,
    /// named as they appear in a diagnostic.
    pub(crate) transformed_slots: Vec<&'static str>,
}

/// Every material's authored PBR declaration, keyed by JSON material index.
///
/// Only materials that say something beyond base colour appear in the map; a
/// missing entry is "no authored PBR", which is exactly the `pbr: None` case.
pub(crate) fn pbr_declarations(document: &serde_json::Value) -> BTreeMap<usize, PbrDecl> {
    let mut declarations = BTreeMap::new();
    let Some(entries) = document
        .get("materials")
        .and_then(serde_json::Value::as_array)
    else {
        return declarations;
    };
    for (index, material) in entries.iter().enumerate() {
        let metallic_roughness = material.get("pbrMetallicRoughness");
        let non_zero_emissive = material
            .get("emissiveFactor")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|components| {
                components
                    .iter()
                    .any(|component| component.as_f64().is_some_and(|value| value != 0.0))
            });
        let declared = metallic_roughness
            .and_then(|pbr| pbr.get("metallicFactor"))
            .is_some()
            || metallic_roughness
                .and_then(|pbr| pbr.get("roughnessFactor"))
                .is_some()
            || metallic_roughness
                .and_then(|pbr| pbr.get("metallicRoughnessTexture"))
                .is_some()
            || material.get("normalTexture").is_some()
            || material.get("occlusionTexture").is_some()
            || material.get("emissiveTexture").is_some()
            || non_zero_emissive;
        let emissive_strength = material
            .get("extensions")
            .and_then(|extensions| extensions.get("KHR_materials_emissive_strength"))
            .is_some();
        // The four slots as the document spells them, with the short name a
        // diagnostic uses. `metallicRoughnessTexture` nests one level deeper.
        let slots: [(&'static str, Option<&serde_json::Value>); 4] = [
            (
                "metallicRoughnessTexture",
                metallic_roughness.and_then(|pbr| pbr.get("metallicRoughnessTexture")),
            ),
            ("normalTexture", material.get("normalTexture")),
            ("emissiveTexture", material.get("emissiveTexture")),
            ("occlusionTexture", material.get("occlusionTexture")),
        ];
        let transformed_slots: Vec<&'static str> = slots
            .into_iter()
            .filter(|(_, info)| {
                info.and_then(|info| info.get("extensions"))
                    .and_then(|extensions| extensions.get("KHR_texture_transform"))
                    .is_some()
            })
            .map(|(slot, _)| slot)
            .collect();
        if !declared && !emissive_strength && transformed_slots.is_empty() {
            continue;
        }
        declarations.insert(
            index,
            PbrDecl {
                declared,
                emissive_strength,
                transformed_slots,
            },
        );
    }
    declarations
}
