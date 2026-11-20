//! SketchUp textured-material import (ADR-0038 A3b).
//!
//! `skpwright` 0.6.0 hands back everything the import needs: a
//! `Material::Textured`'s embedded image bytes, its applied size in inches, and
//! a per-face §4v projective transform (`MeshFace::uv_xform`). This module turns
//! that into the two things the database contract wants:
//!
//! * **one document asset per image**, via `MeshImport::register_texture`
//!   (content addressed, deduped, budgeted — ADR-0038 D5), cached per material
//!   slot so a material shared by many placed runs is registered and hashed
//!   once;
//! * **one §4v UV transform per face**, evaluated later on each world vertex in
//!   metres. A group carries exactly one texture, so if any of its faces has no
//!   usable transform the whole group stays untextured instead of sampling the
//!   wrong texels on that face.
//!
//! Nothing is invented: a material whose image is only an external path, whose
//! bytes were refused by a budget, or that declares no applied size, yields
//! `None` plus a counter the caller reports. Back faces are not imported; the
//! caller counts them separately.

use std::collections::BTreeMap;

use cad_import_core::MeshImport;
use skpwright::{MeshFace, Model, Side, UvXform};

/// Whether `slot` names a `Textured` material (embedded image or external path).
pub(crate) fn is_textured_material(model: &Model, slot: u16) -> bool {
    matches!(
        model.material_of(slot),
        Some(skpwright::Material::Textured { .. })
    )
}

/// Texture keys registered so far, per material slot.
///
/// `None` caches "this slot has no usable texture", so a refusal or an external
/// path is not re-hashed, re-reported or re-counted for every instance that uses
/// the material.
#[derive(Default)]
pub(crate) struct TextureSlots {
    by_slot: BTreeMap<u16, Option<String>>,
}

/// Everything one material group needs to know about its texture.
pub(crate) struct GroupTexture {
    /// The registered document key; `None` keeps the group untextured.
    pub key: Option<String>,
    /// One §4v transform per face, parallel to the group's face list. Every entry
    /// is `Some` exactly when `key` is `Some`.
    pub xforms: Vec<Option<UvXform>>,
    /// Textured faces that therefore stay untextured (no transform or no applied
    /// size).
    pub uv_unavailable: usize,
    /// The material references an image file outside the model.
    pub external_image: bool,
    /// A registration refusal for this material, verbatim (carries its
    /// `import.texture.*` marker).
    pub refusal: Option<String>,
    /// The material is `Textured` at all, so a group left untextured is a loss
    /// worth reporting rather than a plain solid.
    pub material_is_textured: bool,
}

/// Resolve one material group's texture key and its per-face §4v transforms.
pub(crate) fn plan_group(
    model: &Model,
    slot: u16,
    faces: &[&MeshFace],
    import: &mut MeshImport,
    slots: &mut TextureSlots,
) -> GroupTexture {
    let material_is_textured = is_textured_material(model, slot);
    // The model's linked applied size is authoritative; the material's own field
    // is the fallback `skpwright` documents. It is the §4v denominator, so a
    // missing, zero or non-finite size means no mapping at all — and then there
    // is no point registering an image no mesh could sample.
    let applied_size = model
        .applied_size_of(slot)
        .or_else(|| match model.material_of(slot) {
            Some(skpwright::Material::Textured {
                applied_size_in: Some(size),
                ..
            }) => Some(*size),
            _ => None,
        })
        .filter(|size| size.0.is_finite() && size.1.is_finite() && size.0 > 0.0 && size.1 > 0.0);
    if applied_size.is_none() {
        return GroupTexture {
            key: None,
            xforms: vec![None; faces.len()],
            uv_unavailable: if material_is_textured { faces.len() } else { 0 },
            external_image: false,
            refusal: None,
            material_is_textured,
        };
    }

    let mut external_image = false;
    let mut refusal = None;
    let key = match slots.by_slot.get(&slot) {
        Some(cached) => cached.clone(),
        None => {
            let resolved = match model.material_of(slot) {
                Some(skpwright::Material::Textured {
                    image_bytes: Some(bytes),
                    ..
                }) => match import.register_texture(bytes) {
                    Ok(key) => Some(key),
                    // A named refusal leaves the material untextured; the group
                    // keeps its average colour and the caller reports the marker.
                    Err(error) => {
                        refusal = Some(error.to_string());
                        None
                    }
                },
                Some(skpwright::Material::Textured { .. }) => {
                    external_image = true;
                    None
                }
                _ => None,
            };
            slots.by_slot.insert(slot, resolved.clone());
            resolved
        }
    };

    let size = applied_size.expect("checked above");
    let registered = key.is_some();
    let mut xforms: Vec<Option<UvXform>> = if registered {
        faces
            .iter()
            .map(|face| face.uv_xform(Side::Front, size))
            .collect()
    } else {
        vec![None; faces.len()]
    };
    // A group carries exactly one texture: if any face cannot be placed, the
    // whole group drops the UVs and the key (never a partial sample).
    let key = if registered && xforms.iter().all(Option::is_some) {
        key
    } else {
        xforms = vec![None; faces.len()];
        None
    };
    GroupTexture {
        uv_unavailable: if registered && key.is_none() {
            faces.len()
        } else {
            0
        },
        key,
        xforms,
        external_image,
        refusal,
        material_is_textured,
    }
}

/// Whether a face's back material is textured (back faces are not imported).
pub(crate) fn has_textured_back(model: &Model, face: &MeshFace) -> bool {
    face.back_material
        .filter(|slot| *slot != 0)
        .is_some_and(|slot| is_textured_material(model, slot))
}
