//! The per-entry payload codec: one RFC 8878 zstd frame per modelled JSON entry.
//!
//! The container is still zip. Every entry this build writes is a zip **`Stored`**
//! entry, so the entry bytes are exactly the bytes the manifest describes, and the
//! manifest's `codec` field says how to read them:
//!
//! * `zstd` — the payload is one complete zstd frame whose decompressed bytes are
//!   the entry's JSON (`document.json`, `tables/*.json`, `elements/*.json`, …);
//! * `raw` — the payload is the bytes verbatim (`textures/<key>`,
//!   `resources/<kind>/<key>`);
//! * `opaque` — a foreign/unmodelled entry, never decoded here, only copied.
//!
//! # Why a buffered frame
//!
//! `Body::encode` already materialises one entry's JSON as a `Vec`, so the frame
//! is compressed from an in-memory slice: no streaming encoder state is needed,
//! and the frame header can **pledge the content size** (`len`, the logical JSON
//! length). A reader therefore learns the exact output length from the frame
//! header and can allocate precisely that much before decoding — a lying header
//! cannot drive the allocation, because [`decode_frame`] compares the declared
//! size against the manifest before any block is decoded.
//!
//! The cost is one extra buffer per save: the JSON bytes and the frame that
//! replaces them coexist until the JSON is dropped. A future streaming path that
//! fed the zstd frames directly into the zip writer would save roughly one frame
//! buffer at a time; until then, `write::encode` drops the JSON before the frame
//! is handed to the sink, so the peak is bounded by `json + frame` for one entry.
//!
//! # Format constants
//!
//! [`ZSTD_LEVEL`] is part of the format, not a tuning knob: the archive's bytes
//! are a function of the pinned `structured-zstd` version, this level, the
//! enabled features and the input. Two saves of one snapshot must stay
//! byte-identical, so changing either the version or the level changes the format.

use std::io::Write as _;

use structured_zstd::decoding::{read_frame_content_size, FrameContentSize, FrameDecoder};
use structured_zstd::encoding::{CompressionContext, CompressionLevel, StreamingEncoder};

use cad_db::StoreError;

use crate::json::malformed;

/// The compression level every entry frame is written at.
///
/// `6` is a middle level: JSON compresses well and the save path is not the
/// bottleneck. It is fixed forever for this format version — changing it changes
/// every archive's bytes.
pub(crate) const ZSTD_LEVEL: i32 = 6;

/// The context string for frame-level failures.
const CONTEXT: &str = "zstd";

/// One reusable zstd frame encoder for a whole save.
///
/// The [`CompressionContext`] is created once and reused for every entry: it
/// keeps the match-finder tables between frames, so a document with many shards
/// does not rebuild them per entry. Reusing it is byte-identical to a fresh
/// context per frame (the codec's documented contract), which keeps the save
/// deterministic.
pub(crate) struct FrameEncoder {
    context: CompressionContext,
}

impl FrameEncoder {
    /// A frame encoder at the format's fixed level.
    pub(crate) fn new() -> FrameEncoder {
        FrameEncoder {
            context: CompressionContext::new(CompressionLevel::from_level(ZSTD_LEVEL)),
        }
    }

    /// Compress `bytes` into one complete frame with a pledged content size.
    pub(crate) fn frame(&mut self, bytes: &[u8]) -> Result<Vec<u8>, StoreError> {
        // Borrowing the context for one frame leaves it ready for the next; the
        // frame belongs to the new drain (the encoder's `Drop` abandons a frame
        // that was not finished, so a failure cannot leak into the next entry).
        let mut encoder = StreamingEncoder::with_context(Vec::new(), &mut self.context);
        encoder
            .set_pledged_content_size(bytes.len() as u64)
            .map_err(|error| malformed(CONTEXT, error))?;
        encoder
            .write_all(bytes)
            .map_err(|error| malformed(CONTEXT, error))?;
        encoder.finish().map_err(|error| malformed(CONTEXT, error))
    }
}

/// Decode one `codec: "zstd"` entry payload.
///
/// `len` is the manifest's logical length. The frame header must declare exactly
/// that size before any block is decoded; the output buffer is then allocated at
/// exactly `len` and the decoder may not grow it (a caller-sized slice, not a
/// streaming `read_to_end`). A corrupt header therefore fails the comparison
/// instead of driving the allocation.
pub(crate) fn decode_frame(payload: &[u8], len: u64, path: &str) -> Result<Vec<u8>, StoreError> {
    match read_frame_content_size(payload).map_err(|error| malformed(path, error))? {
        FrameContentSize::Known(declared) if declared == len => {}
        FrameContentSize::Known(declared) => {
            return Err(malformed(
                path,
                format!("the zstd frame declares {declared} bytes, manifest.json records {len}"),
            ))
        }
        FrameContentSize::Unknown => {
            return Err(malformed(
                path,
                "the zstd frame declares no content size, so manifest.json cannot bound it",
            ))
        }
    }
    let size = usize::try_from(len).map_err(|_| {
        malformed(
            path,
            format!("{len} bytes cannot be addressed on this platform"),
        )
    })?;
    let mut decoded = vec![0u8; size];
    let written = FrameDecoder::new()
        .decode_all(payload, &mut decoded)
        .map_err(|error| malformed(path, error))?;
    if written != size {
        return Err(malformed(
            path,
            format!("the zstd frame decoded to {written} bytes, manifest.json records {len}"),
        ));
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_declares_its_logical_size_and_round_trips() {
        let bytes = br#"{"a":[1,2,3],"b":"x"}"#;
        let mut encoder = FrameEncoder::new();
        let frame = encoder.frame(bytes).expect("compress");
        assert_eq!(
            read_frame_content_size(&frame).expect("header"),
            FrameContentSize::Known(bytes.len() as u64)
        );
        assert_eq!(
            decode_frame(&frame, bytes.len() as u64, "document.json").expect("decode"),
            bytes
        );
    }

    #[test]
    fn an_empty_frame_round_trips() {
        let mut encoder = FrameEncoder::new();
        let frame = encoder.frame(b"").expect("compress");
        assert_eq!(
            read_frame_content_size(&frame).expect("header"),
            FrameContentSize::Known(0)
        );
        assert_eq!(
            decode_frame(&frame, 0, "document.json").expect("decode"),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn encoding_is_deterministic_across_a_reused_context() {
        let bytes = br#"{"id":7,"class":"cad:Line"}"#;
        let mut first = FrameEncoder::new();
        let a = first.frame(bytes).expect("compress");
        let b = first.frame(bytes).expect("compress");
        assert_eq!(a, b, "one context must produce identical frames");
        assert_eq!(
            a,
            FrameEncoder::new().frame(bytes).expect("compress"),
            "a reused context must match a fresh one"
        );
    }

    #[test]
    fn a_header_that_lies_about_its_size_is_refused_before_decoding() {
        let bytes = b"some json payload";
        let mut encoder = FrameEncoder::new();
        let frame = encoder.frame(bytes).expect("compress");
        let error = decode_frame(&frame, bytes.len() as u64 + 1, "document.json")
            .expect_err("a mismatched declared size");
        assert_eq!(
            error.marker,
            cad_domain::ymodel::markers::INVALID_DOCUMENT,
            "{error}"
        );
    }

    #[test]
    fn garbage_is_not_a_frame() {
        let error = decode_frame(b"not a zstd frame", 0, "document.json").expect_err("garbage");
        assert_eq!(
            error.marker,
            cad_domain::ymodel::markers::INVALID_DOCUMENT,
            "{error}"
        );
    }
}
