//! glTF buffer resolution without the feature-gated `gltf::import_buffers`.
//!
//! A bytes-only import can resolve exactly two sources: the GLB BIN chunk
//! (`Source::Bin`) and an embedded base64 `data:` URI. Any other URI names a
//! file this adapter cannot read, so it is a stable [`CadError::Unsupported`]
//! refusal instead of a silently empty buffer.

use base64::Engine as _;
use cad_domain::{CadError, CadResult};
use gltf::buffer::Source;

use crate::{DIAG_BUFFER_BASE64, DIAG_BUFFER_LENGTH, DIAG_EXTERNAL_BUFFER, DIAG_MISSING_BIN};

/// One resolved buffer: borrowed from the GLB blob, or decoded from a data URI.
pub(crate) enum ResolvedBuffer<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
}

impl ResolvedBuffer<'_> {
    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            ResolvedBuffer::Borrowed(bytes) => bytes,
            ResolvedBuffer::Owned(bytes) => bytes,
        }
    }
}

/// Resolve every declared buffer, refusing anything the import cannot read.
pub(crate) fn resolve_buffers(gltf: &gltf::Gltf) -> CadResult<Vec<ResolvedBuffer<'_>>> {
    let mut buffers = Vec::with_capacity(gltf.document.buffers().len());
    for buffer in gltf.document.buffers() {
        let resolved = match buffer.source() {
            Source::Bin => ResolvedBuffer::Borrowed(gltf.blob.as_deref().ok_or_else(|| {
                CadError::CorruptData(format!(
                    "{DIAG_MISSING_BIN}: the GLB declares a binary buffer but has no BIN chunk"
                ))
            })?),
            Source::Uri(uri) => match data_uri_payload(uri) {
                Some(payload) => ResolvedBuffer::Owned(
                    base64::engine::general_purpose::STANDARD
                        .decode(payload)
                        .map_err(|error| {
                            CadError::CorruptData(format!(
                                "{DIAG_BUFFER_BASE64}: a data URI buffer is not valid base64: \
                                 {error}"
                            ))
                        })?,
                ),
                None => {
                    return Err(CadError::Unsupported(format!(
                        "{DIAG_EXTERNAL_BUFFER}: the document references an external buffer \
                         `{uri}`; this bytes-only import cannot read it"
                    )));
                }
            },
        };
        let declared = buffer.length();
        if resolved.as_slice().len() < declared {
            return Err(CadError::CorruptData(format!(
                "{DIAG_BUFFER_LENGTH}: buffer {} declares {declared} byte(s) but only {} were \
                 resolved",
                buffer.index(),
                resolved.as_slice().len()
            )));
        }
        buffers.push(resolved);
    }
    Ok(buffers)
}

/// The base64 payload of a `data:...;base64,<payload>` URI.
///
/// A non-base64 data URI (percent-encoded) is not decoded either: it is treated
/// like any other unreadable URI so the caller refuses it by name.
fn data_uri_payload(uri: &str) -> Option<&str> {
    let rest = uri.strip_prefix("data:")?;
    let (metadata, payload) = rest.split_once(',')?;
    let is_base64 = metadata
        .rsplit(';')
        .next()
        .is_some_and(|part| part.eq_ignore_ascii_case("base64"));
    is_base64.then_some(payload)
}
