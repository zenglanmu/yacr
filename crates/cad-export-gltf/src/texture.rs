//! UV0 and texture emission for the glTF writer (ADR-0038 D11, ADR-0039 D5).
//!
//! Every image the writer emits is a document-owned texture from `cad-db`:
//! `images[]` points into the single output buffer (so `.gltf` stays one data-URI
//! buffer and GLB stays one BIN chunk), `textures[]`/`samplers[]` stay 1:1 with
//! the distinct content keys, and a primitive only claims `TEXCOORD_0` when it
//! really references an image.
//!
//! The base-colour slot and the four PBR slots share this module so one rule
//! decides whether a texture can be written at all: the key must promise the
//! bytes' container, the mesh must have a writable UV0, and identical bytes must
//! resolve to one image. When any of those fails the channel is omitted and
//! reported as `export.gltf.texture_not_written` — never written with invented
//! bytes and never silently dropped.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::collect::SourceMesh;
use crate::report::ReportBuilder;
use crate::write::{Builder, TEXTURE_NOT_WRITTEN};

/// The 8-byte PNG signature (ISO/IEC 15948 §5.2).
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// The 3-byte JPEG SOI + first marker prefix.
const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];

/// UV0 narrowed to `f32` (ADR-0038 D1).
///
/// The frame rotation is deliberately **not** applied: `(u, v)` is a surface
/// parameterisation, so rotating or mirroring a position must not re-map the
/// image. Narrowing at this boundary is the only precision step.
fn uvs(mesh: &SourceMesh) -> Vec<[f32; 2]> {
    mesh.uvs
        .iter()
        .map(|[u, v]| [*u as f32, *v as f32])
        .collect()
}

/// The glTF `images[].mimeType` a texture key promises.
///
/// The key is the only place the format is declared, so the stored bytes are
/// checked against it: writing a PNG under `image/jpeg` would make the file
/// undecodable while still looking complete. A mismatch is `None` and the
/// writer reports the texture instead of writing it.
pub(crate) fn texture_mime(key: &str, bytes: &[u8]) -> Option<&'static str> {
    if key.ends_with(".png") {
        bytes.starts_with(&PNG_MAGIC).then_some("image/png")
    } else if key.ends_with(".jpg") {
        bytes.starts_with(&JPEG_MAGIC).then_some("image/jpeg")
    } else {
        None
    }
}

/// UV0 as far as the writer can store it.
///
/// The reason a set is unusable is kept apart from the failure so the diagnostic
/// can name it exactly; `TEXCOORD_0` is only ever written for [`Uv0::Usable`].
pub(crate) enum Uv0 {
    /// One finite pair per vertex, narrowed to `f32`.
    Usable(Vec<[f32; 2]>),
    /// The source declared UV0 that does not pair with its vertex list.
    Dropped,
    /// The primitive has no UV0 at all.
    Absent,
    /// A declared UV0 component is outside the `f32` range.
    NonFinite,
}

impl Uv0 {
    pub(crate) fn of(mesh: &SourceMesh) -> Self {
        if mesh.uvs_dropped {
            return Uv0::Dropped;
        }
        if mesh.uvs.is_empty() {
            return Uv0::Absent;
        }
        let uvs = uvs(mesh);
        if uvs
            .iter()
            .any(|uv| uv.iter().any(|value| !value.is_finite()))
        {
            return Uv0::NonFinite;
        }
        Uv0::Usable(uvs)
    }

    pub(crate) fn pairs(&self) -> Option<&[[f32; 2]]> {
        match self {
            Uv0::Usable(pairs) => Some(pairs),
            Uv0::Dropped | Uv0::Absent | Uv0::NonFinite => None,
        }
    }
}

/// Register the mesh's base-colour image and return its `textures[]` index.
///
/// `None` after reporting one `texture_not_written` conversion when the source
/// carried UV0 or named a texture that cannot be written. The caller adds
/// `TEXCOORD_0`, because the accessor belongs in the primitive's `attributes`
/// map beside `POSITION` and is shared with any PBR texture channel.
pub(crate) fn base_colour_texture(
    mesh: &SourceMesh,
    uv0: &Uv0,
    textures: &mut Textures,
    builder: &mut Builder,
    report: &mut ReportBuilder,
) -> Option<usize> {
    let lost = |report: &mut ReportBuilder, reason: String| {
        report.converted(
            TEXTURE_NOT_WRITTEN,
            format!("entity #{} {reason}", mesh.object.0),
        );
    };
    if matches!(uv0, Uv0::Dropped) {
        lost(
            report,
            "declares UV0 that does not pair with its vertex list; TEXCOORD_0 was not written"
                .to_string(),
        );
        return None;
    }
    let Some(texture) = &mesh.texture else {
        if let Some(key) = &mesh.missing_texture {
            lost(
                report,
                format!(
                    "names texture {key:?} but the document does not carry its bytes; TEXCOORD_0 \
                     and the image were not written"
                ),
            );
        }
        // UV0 that nothing samples is reported by the caller, once it knows
        // whether a PBR channel wrote `TEXCOORD_0`.
        return None;
    };
    if matches!(uv0, Uv0::Absent) {
        lost(
            report,
            format!(
                "names texture {:?} but has no UV0; the image and TEXCOORD_0 were not written",
                texture.key
            ),
        );
        return None;
    }
    let Some(mime) = texture_mime(&texture.key, &texture.bytes) else {
        lost(
            report,
            format!(
                "texture {:?} does not carry bytes matching its extension; TEXCOORD_0 and the \
                 image were not written",
                texture.key
            ),
        );
        return None;
    };
    if matches!(uv0, Uv0::NonFinite) {
        lost(
            report,
            format!(
                "texture {:?} has UV0 outside the f32 range; TEXCOORD_0 and the image were not \
                 written",
                texture.key
            ),
        );
        return None;
    }
    Some(textures.index_for(builder, &texture.key, &texture.bytes, mime))
}

/// `images[]` / `textures[]` for the distinct texture keys, in first-use order.
///
/// The traversal is the entity-ascending collect order, so the emitted order is
/// stated rather than incidental; the key index is a map lookup, never a pointer
/// or hash of one. Image and texture indices are 1:1 because a distinct key
/// produces exactly one image and one texture.
#[derive(Default)]
pub(crate) struct Textures {
    pub(crate) images: Vec<Value>,
    pub(crate) textures: Vec<Value>,
    indices: BTreeMap<String, usize>,
}

impl Textures {
    /// The `textures[]` index for `key`, adding its image once.
    pub(crate) fn index_for(
        &mut self,
        builder: &mut Builder,
        key: &str,
        bytes: &[u8],
        mime: &'static str,
    ) -> usize {
        if let Some(index) = self.indices.get(key) {
            return *index;
        }
        // Image bufferViews carry no `target`: the glTF schema reserves `target`
        // for accessor data, and an image view with one is invalid.
        let view = builder.push_view(bytes, None);
        let image = self.images.len();
        self.images.push(json!({
            "bufferView": view,
            "mimeType": mime,
        }));
        let texture = self.textures.len();
        self.textures.push(json!({
            "sampler": 0,
            "source": image,
        }));
        self.indices.insert(key.to_string(), texture);
        texture
    }

    /// Whether any texture was written (so the sampler/texture arrays exist).
    pub(crate) fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}
