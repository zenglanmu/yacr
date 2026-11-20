//! Base-colour texture resolution for the glTF importer (ADR-0038 D6).
//!
//! A bytes-only import can read exactly two image sources: bytes the document
//! itself carries (a `bufferView` into a resolved buffer) and an embedded base64
//! `data:` URI. An external file is named and reported, never fetched and never
//! replaced with a placeholder.
//!
//! Only the **base-colour** slot is imported. The document-level losses that come
//! with that scope find their stable diagnostics here too: the ignored
//! `KHR_texture_transform`, and one note naming every other texture channel the
//! document carries.

use std::borrow::Cow;
use std::collections::BTreeSet;

use base64::Engine as _;
use cad_domain::{CadError, CadResult, Diagnostic};
use gltf::{image::Source as ImageSource, texture::Info as TextureInfo};

use crate::buffers::{data_uri_payload, ResolvedBuffer};
use crate::{DIAG_IMAGE_BASE64, DIAG_IMAGE_OUT_OF_RANGE, DIAG_TEXTURE_CHANNEL_NOT_IMPORTED};

/// The bytes of one base-colour texture, or why they are not part of the import.
pub(crate) enum BaseColourTexture<'a> {
    /// Encoded PNG/JPEG bytes carried by the document itself.
    Embedded(Cow<'a, [u8]>),
    /// The image names an external file (or a non-base64 data URI).
    Unavailable { uri: String },
}

/// Resolve a base-colour texture's encoded bytes without touching the filesystem.
///
/// A `bufferView` image is sliced out of its already-resolved buffer; a URI image
/// is decoded only when it is an embedded base64 `data:` URI. Anything else is an
/// external file this bytes-only import cannot read, which the caller reports
/// instead of guessing at.
///
/// `gltf::Gltf::from_slice` validates the document before this runs, so the
/// `unwrap`s inside `Image::source()` (a `bufferView` image must name an existing
/// view and a MIME type) are only reachable for a document that bypassed
/// validation. This reader relies on that gate rather than catching the panic: an
/// invalid document is a parse failure, not a loss to report.
pub(crate) fn resolve_base_colour_texture<'a>(
    info: &TextureInfo<'a>,
    buffers: &'a [ResolvedBuffer<'a>],
) -> CadResult<BaseColourTexture<'a>> {
    let image = info.texture().source();
    match image.source() {
        // The declared MIME type is not trusted: `register_texture` sniffs the
        // container, so a `.png` key can never name JPEG bytes.
        ImageSource::View { view, .. } => {
            let start = view.offset();
            let end = start.checked_add(view.length()).ok_or_else(|| {
                CadError::CorruptData(format!(
                    "{DIAG_IMAGE_OUT_OF_RANGE}: the base-color image's buffer view has a length \
                     that overflows a usize"
                ))
            })?;
            let buffer = buffers
                .get(view.buffer().index())
                .ok_or_else(|| {
                    CadError::CorruptData(format!(
                        "{DIAG_IMAGE_OUT_OF_RANGE}: the base-color image's buffer view names \
                         buffer {}, which the document does not declare",
                        view.buffer().index()
                    ))
                })?
                .as_slice();
            let bytes = buffer.get(start..end).ok_or_else(|| {
                CadError::CorruptData(format!(
                    "{DIAG_IMAGE_OUT_OF_RANGE}: the base-color image's buffer view {start}..{end} \
                     is outside a {}-byte buffer",
                    buffer.len()
                ))
            })?;
            Ok(BaseColourTexture::Embedded(Cow::Borrowed(bytes)))
        }
        ImageSource::Uri { uri, .. } => match data_uri_payload(uri) {
            Some(payload) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(payload)
                    .map_err(|error| {
                        CadError::CorruptData(format!(
                            "{DIAG_IMAGE_BASE64}: the base-color image's data URI is not valid \
                             base64: {error}"
                        ))
                    })?;
                Ok(BaseColourTexture::Embedded(Cow::Owned(bytes)))
            }
            // An external file, or a data URI that is not base64: both name bytes
            // this import cannot read.
            None => Ok(BaseColourTexture::Unavailable {
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

/// The materials whose **base-colour** texture info carries
/// `KHR_texture_transform`.
///
/// The `gltf` crate here is built without its `KHR_texture_transform` and
/// `extensions` features, so `texture::Info::texture_transform()` does not exist.
/// The document JSON is inspected instead — the same parse the version check
/// needs anyway, so this costs no extra parse. Only the base-colour slot counts:
/// a transform on a normal map's info is part of that channel's own loss
/// ([`DIAG_TEXTURE_CHANNEL_NOT_IMPORTED`]), not this one.
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

/// Report the texture channels this importer does not read, once per document.
///
/// Only the base-colour slot is imported (ADR-0038 §7), so a document carrying a
/// normal/occlusion/emissive/metallicRoughness map loses those channels. The note
/// names them instead of leaving the omission silent; it is a note rather than
/// `Partial` because every one of those channels is outside the ADR's scope for
/// *every* document, not a failure of this particular import.
pub(crate) fn channel_note(document: &gltf::Document) -> Option<Diagnostic> {
    let (mut normal, mut occlusion, mut emissive, mut metallic_roughness) =
        (false, false, false, false);
    for material in document.materials() {
        normal |= material.normal_texture().is_some();
        occlusion |= material.occlusion_texture().is_some();
        emissive |= material.emissive_texture().is_some();
        metallic_roughness |= material
            .pbr_metallic_roughness()
            .metallic_roughness_texture()
            .is_some();
    }
    let channels: Vec<&str> = [
        (normal, "normal"),
        (occlusion, "occlusion"),
        (emissive, "emissive"),
        (metallic_roughness, "metallicRoughness"),
    ]
    .into_iter()
    .filter_map(|(present, name)| present.then_some(name))
    .collect();
    if channels.is_empty() {
        return None;
    }
    Some(Diagnostic {
        object: None,
        code: DIAG_TEXTURE_CHANNEL_NOT_IMPORTED.into(),
        message: format!(
            "only the base-color texture is imported; the document also carries {} texture \
             channel(s), which are not",
            channels.join(", ")
        ),
    })
}
