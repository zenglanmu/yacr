//! IFC base-colour texture import (ADR-0038 A3b).
//!
//! `ifc-lite-geometry` owns the whole IFC-side resolution: `build_texture_index`
//! reads `IfcIndexedTriangleTextureMap` / `IfcTextureVertexList` and decodes
//! `IfcBlobTexture` / `IfcPixelTexture` to RGBA8, and
//! `process_element_with_submeshes_textured` turns a textured face set into its
//! own sub-mesh carrying per-vertex UVs plus a pixel source. Three things are
//! this adapter's, and they live here so `lib.rs` stays a walk:
//!
//! * **Which elements may take the textured router.** Upstream's void path never
//!   passes a texture index ("a CSG cut rebuilds vertices, which would orphan
//!   the UVs"), so an element that hosts an `IfcRelVoidsElement` must stay on the
//!   void path and report [`DIAG_TEXTURED_VOID_FALLBACK`].
//! * **Registering the image as a document asset.** A decoded RGBA8 image is
//!   encoded to PNG once and handed to `MeshImport::register_texture`, whose
//!   content-addressed key the mesh carries (ADR-0038 D5). One cache per import
//!   means one encode per `IfcSurfaceTexture` entity however many face sets use
//!   it.
//! * **UV transfer.** `SubMesh.uvs` is a flat `f32` pair list that must be 1:1
//!   with the sub-mesh's positions; the domain stores `f64` pairs. A short,
//!   long or non-finite list is dropped whole ([`DIAG_UVS_DROPPED`]) rather than
//!   sampled against the wrong vertices.
//!
//! Nothing here invents a texture: an `IfcImageTexture` names a file outside the
//! model and is reported ([`DIAG_EXTERNAL_TEXTURE_UNAVAILABLE`]), never read.

use std::collections::BTreeMap;

use cad_connector_core::MeshImport;
use cad_domain::Diagnostic;
use ifc_lite_core::EntityDecoder;
use ifc_lite_geometry::{TextureAttachment, TextureSource};

/// Stable diagnostic code (Partial): an `IfcImageTexture` points at an image
/// file outside the model, which this adapter does not read.
pub const DIAG_EXTERNAL_TEXTURE_UNAVAILABLE: &str = "import.ifc.external_texture_unavailable";
/// Stable diagnostic code (Partial): a textured product is cut by an opening, and
/// the upstream void path cannot carry UVs through a CSG reconstruction.
pub const DIAG_TEXTURED_VOID_FALLBACK: &str = "import.ifc.textured_void_fallback";
/// Stable diagnostic code (Partial): a decoded image could not be encoded to PNG,
/// so the mesh is stored untextured.
pub const DIAG_TEXTURE_ENCODE_FAILED: &str = "import.ifc.texture_encode_failed";
/// Stable diagnostic code (Partial): the texture coordinates did not pair 1:1 with
/// the sub-mesh's vertices, so the whole UV set (and the texture reference) was
/// dropped.
pub const DIAG_UVS_DROPPED: &str = "import.ifc.uvs_dropped";

/// Encode a decoded RGBA8 image as an 8-bit RGBA PNG.
///
/// The output is deterministic for the same pixels: fixed colour type/depth, no
/// timestamps, no text chunks, and the crate's default filter/compression
/// choice. That matters because the key is the SHA-256 of these bytes, so the
/// same IFC image must produce the same key on every run.
pub(crate) fn encode_rgba_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 {
        return Err(format!(
            "the image is {width}x{height}, so it has no pixels"
        ));
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("{width}x{height} RGBA does not fit a usize"))?;
    if rgba.len() != expected {
        return Err(format!(
            "the decoder produced {} byte(s) for a {width}x{height} RGBA image, expected {expected}",
            rgba.len()
        ));
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Balanced);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(rgba)
            .map_err(|error| error.to_string())?;
        writer.finish().map_err(|error| error.to_string())?;
    }
    Ok(bytes)
}

/// Convert one sub-mesh's flat `f32` UV pairs to the domain's `f64` UV0.
///
/// `None` when the list is not exactly two components per vertex or holds a
/// non-finite value: the mesh then keeps its geometry and loses the texture,
/// which is what [`DIAG_UVS_DROPPED`] reports.
pub(crate) fn uvs_from_f32(uvs: &[f32], vertex_count: usize) -> Option<Vec<[f64; 2]>> {
    if vertex_count == 0 || uvs.len() != vertex_count * 2 {
        return None;
    }
    let mut out = Vec::with_capacity(vertex_count);
    for pair in uvs.as_chunks::<2>().0 {
        let uv = [pair[0] as f64, pair[1] as f64];
        if !uv[0].is_finite() || !uv[1].is_finite() {
            return None;
        }
        out.push(uv);
    }
    Some(out)
}

/// Document texture keys resolved so far, keyed by `IfcSurfaceTexture` express id.
///
/// `None` means "this texture cannot be used" (external file, refused budget,
/// encode failure) and is cached too, so one bad texture is reported once, not
/// once per face set that maps it.
#[derive(Default)]
pub(crate) struct TextureCache {
    keys: BTreeMap<u32, Option<String>>,
}

impl TextureCache {
    /// The registered key for `attachment`, or `None` when the image cannot enter
    /// the document (already reported by name).
    pub(crate) fn key_for(
        &mut self,
        attachment: &TextureAttachment,
        label: &str,
        import: &mut MeshImport,
    ) -> Option<String> {
        if let Some(cached) = self.keys.get(&attachment.texture_id) {
            return cached.clone();
        }
        let key = match &attachment.source {
            TextureSource::Decoded(texture) => {
                match encode_rgba_png(&texture.rgba, texture.width, texture.height) {
                    Ok(png) => match import.register_texture(&png) {
                        Ok(key) => Some(key),
                        Err(error) => {
                            // A budget/format refusal is a named loss, not a
                            // failure of the import: the mesh keeps its geometry.
                            let message = error.to_string();
                            let code = marker_of(&message);
                            import.record(
                                Diagnostic {
                                    object: None,
                                    code,
                                    message: format!(
                                        "IFC product {label}: texture #{} was not stored: {message}",
                                        attachment.texture_id
                                    ),
                                },
                                Some(format!(
                                    "an IFC texture of product {label} was refused and the mesh \
                                     is untextured"
                                )),
                            );
                            None
                        }
                    },
                    Err(reason) => {
                        import.record(
                            Diagnostic {
                                object: None,
                                code: DIAG_TEXTURE_ENCODE_FAILED.into(),
                                message: format!(
                                    "IFC product {label}: the decoded {}x{} texture #{} could not \
                                     be encoded to PNG ({reason}), so the mesh is untextured",
                                    texture.width, texture.height, attachment.texture_id
                                ),
                            },
                            Some(format!(
                                "an IFC texture of product {label} could not be encoded to PNG"
                            )),
                        );
                        None
                    }
                }
            }
            TextureSource::Image(reference) => {
                // `IFCIMAGETEXTURE.URLReference` names a sibling file (often in
                // an `.ifcZIP`); its bytes are not in this import and this
                // adapter does not touch the filesystem for them.
                import.record(
                    Diagnostic {
                        object: None,
                        code: DIAG_EXTERNAL_TEXTURE_UNAVAILABLE.into(),
                        message: format!(
                            "IFC product {label}: IfcImageTexture #{} references '{}' outside the \
                             model; the geometry and its texture coordinates are kept, and the \
                             mesh stays untextured",
                            attachment.texture_id, reference.url
                        ),
                    },
                    Some(format!(
                        "an IFC texture of product {label} is an external file and was not read"
                    )),
                );
                None
            }
        };
        self.keys.insert(attachment.texture_id, key.clone());
        key
    }
}

/// The `import.*` marker a [`cad_domain::CadError`] message starts with.
fn marker_of(message: &str) -> String {
    message
        .split(':')
        .next()
        .map(str::trim)
        .filter(|marker| marker.starts_with("import."))
        .unwrap_or("import.texture.refused")
        .to_string()
}

/// Whether any representation item of `element` is a face set `hit` calls
/// textured.
///
/// The texture index is keyed by **face set** id, not by product, so the walk is
/// `IfcProduct.Representation` → `IfcProductDefinitionShape.Representations` →
/// `IfcShapeRepresentation.Items`, one level into an `IfcMappedItem` (a textured
/// face set inside a shared map is a real export shape). A representation this
/// walk cannot decode answers `false`: the element then takes the plain path,
/// exactly as before, instead of claiming a texture it cannot prove.
pub(crate) fn element_has_textured_face_set(
    element: &ifc_lite_core::DecodedEntity,
    decoder: &mut EntityDecoder<'_>,
    hit: &dyn Fn(u32) -> bool,
) -> bool {
    use ifc_lite_core::slot;
    let Some(shape_ref) = element.ref_at(slot::IfcProduct::REPRESENTATION) else {
        return false;
    };
    let Ok(shape) = decoder.decode_by_id(shape_ref) else {
        return false;
    };
    // `IfcProductDefinitionShape` and `IfcProductRepresentation` both carry
    // `Representations` at the same index, so the subtype does not matter.
    let Some(reps) = shape
        .attr(slot::IfcProductDefinitionShape::REPRESENTATIONS)
        .and_then(|value| value.as_list())
    else {
        return false;
    };
    for representation in reps.iter().filter_map(|value| value.as_entity_ref()) {
        for item in representation_items(decoder, representation) {
            if hit(item) {
                return true;
            }
            if mapped_items(decoder, item)
                .iter()
                .any(|nested| hit(*nested))
            {
                return true;
            }
        }
    }
    false
}

/// The `Items` ids of one `IfcShapeRepresentation`.
fn representation_items(decoder: &mut EntityDecoder<'_>, representation: u32) -> Vec<u32> {
    use ifc_lite_core::slot;
    let Ok(entity) = decoder.decode_by_id(representation) else {
        return Vec::new();
    };
    entity
        .attr(slot::IfcShapeRepresentation::ITEMS)
        .and_then(|value| value.as_list())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_entity_ref())
                .collect()
        })
        .unwrap_or_default()
}

/// The source items of one `IfcMappedItem` (`MappingSource` →
/// `IfcRepresentationMap.MappedRepresentation` → `Items`), empty for a non-mapped
/// item.
fn mapped_items(decoder: &mut EntityDecoder<'_>, item: u32) -> Vec<u32> {
    use ifc_lite_core::slot;
    let Ok(entity) = decoder.decode_by_id(item) else {
        return Vec::new();
    };
    if !entity
        .ifc_type
        .as_str()
        .eq_ignore_ascii_case("IFCMAPPEDITEM")
    {
        return Vec::new();
    }
    let Some(map) = entity
        .ref_at(slot::IfcMappedItem::MAPPING_SOURCE)
        .and_then(|map_ref| decoder.decode_by_id(map_ref).ok())
    else {
        return Vec::new();
    };
    let Some(mapped) = map.ref_at(slot::IfcRepresentationMap::MAPPED_REPRESENTATION) else {
        return Vec::new();
    };
    representation_items(decoder, mapped)
}
