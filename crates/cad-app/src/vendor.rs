//! Vendor display-representation data shared by every carrier.
//!
//! The WebView plugin decodes `TCH_*` bytes into primitives; the host never
//! writes them to `cad-db`. This module owns the two carrier-neutral pieces:
//!
//! * the **derived representation store** keyed by the source entity handle, and
//! * the **records frame encoder** (the WebView plugin's input ABI).
//!
//! The result-frame parser lives in [`cad_ui_core::session::vendor`] because it
//! validates the token/generation state machine; the primitive types here are what
//! the scene assembly consumes.
//!
//! # Authority
//!
//! The original `DbEntity` stays authoritative. A store entry is a display cache
//! for one source handle: picking/selection keeps the source entity id, and
//! save/export never sees these primitives. The host builds a new immutable store
//! per successful plugin pass and swaps it in; a document replacement clears it.

use std::collections::HashMap;
use std::sync::Arc;

use cad_domain::Point3;

use crate::VendorRecord;

/// Vendor wire/frame ABI version. Bumped only for a breaking layout change.
///
/// v4 extends the result frame with primitive kind 5 (block insert). The
/// records-frame layout is unchanged from v3.
pub const VENDOR_ABI_VERSION: u32 = 1;

/// Records frame magic (`"YAND"`, little-endian).
pub const RECORDS_MAGIC: u32 = 0x5941_4e44;

/// Upper bound of records copied into one frame.
pub const MAX_VENDOR_RECORDS: usize = 200_000;

/// Upper bound of one records frame.
pub const MAX_RECORDS_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Upper bound of one stream (main/text/handle) copied from a record.
pub const MAX_VENDOR_STREAM_BYTES: usize = 16 * 1024 * 1024;

/// One decoded text primitive (TCH_TEXT / TCH_MTEXT).
///
/// The host shapes this with the same `FontEngine` the normal representation
/// uses; a text primitive is not written to `cad-db`.
#[derive(Debug, Clone, PartialEq)]
pub struct VendorText {
    pub origin: Point3,
    /// World height (already multiplied by the TCH drawing scale).
    pub height: f64,
    /// Radians about `origin`.
    pub rotation: f64,
    /// MTEXT wrap width in world units; `None` keeps written line breaks only.
    pub wrap_width: Option<f64>,
    /// MTEXT attachment numbering, 1..9.
    pub attachment: u8,
    pub text: String,
}

/// One decoded display primitive, in world coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum VendorPrimitive {
    /// A connected polyline (adjacent points are joined).
    Lines(Vec<Point3>),
    /// Independent endpoint pairs (even length).
    Segments(Vec<Point3>),
    /// A circular arc with a signed sweep in radians.
    Arc {
        center: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    /// A full circle.
    Circle { center: Point3, radius: f64 },
    /// Text shaped by the host font engine.
    Text(VendorText),
    /// A reference to an imported block definition, expanded by the host.
    ///
    /// The plugin sees the TCH opening's 2D block handle, never a table name;
    /// the host resolves it through `DrawingDatabase::block_by_source_handle`.
    Block {
        /// Source DWG handle of the block definition.
        block: u64,
        /// Insertion point and transform, already in world coordinates.
        origin: Point3,
        /// Per-axis scale; negative values express the TCH mirror flags.
        scale: Point3,
        /// Rotation about Z in radians.
        rotation: f64,
    },
}

/// All primitives decoded for one source entity handle.
#[derive(Debug, Clone, PartialEq)]
pub struct VendorEntityPrimitives {
    /// The source entity handle (DWG handle, as stored in `source_handle`).
    pub handle: u64,
    pub primitives: Vec<VendorPrimitive>,
}

/// An immutable derived-representation snapshot for one vendor pass.
///
/// `revision` is the pass token; a replacement store always carries a new one, so
/// the scene cache can detect the change without comparing every primitive.
#[derive(Debug, Clone, Default)]
pub struct VendorRepresentationStore {
    revision: u64,
    by_handle: HashMap<u64, Vec<VendorPrimitive>>,
    primitive_count: usize,
}

impl VendorRepresentationStore {
    /// Build a store from one plugin result.
    pub fn new(revision: u64, entities: Vec<VendorEntityPrimitives>) -> Self {
        let mut by_handle: HashMap<u64, Vec<VendorPrimitive>> = HashMap::new();
        let mut primitive_count = 0usize;
        for entity in entities {
            if entity.primitives.is_empty() {
                continue;
            }
            primitive_count = primitive_count.saturating_add(entity.primitives.len());
            // One entity handle per document: a duplicate is a plugin bug, and
            // merging would silently change the intended display. Keep the first
            // non-empty set and ignore a later duplicate.
            by_handle.entry(entity.handle).or_insert(entity.primitives);
        }
        Self {
            revision,
            by_handle,
            primitive_count,
        }
    }

    /// An empty snapshot for a document with no vendor primitives.
    pub fn empty(revision: u64) -> Self {
        Self {
            revision,
            ..Self::default()
        }
    }

    /// The pass token this snapshot belongs to.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Total primitives in the snapshot.
    pub fn primitive_count(&self) -> usize {
        self.primitive_count
    }

    /// The primitives for a `DbEntity::object.source_handle` (uppercase hex).
    pub fn for_source_handle(&self, handle: &str) -> Option<&[VendorPrimitive]> {
        let handle = handle.trim();
        let handle = handle
            .strip_prefix("0x")
            .or_else(|| handle.strip_prefix("0X"))
            .unwrap_or(handle);
        let handle = u64::from_str_radix(handle, 16).ok()?;
        self.by_handle.get(&handle).map(Vec::as_slice)
    }

    /// A replacement snapshot for one pass, keeping the same primitive data.
    pub fn with_revision(&self, revision: u64) -> Self {
        Self {
            revision,
            by_handle: self.by_handle.clone(),
            primitive_count: self.primitive_count,
        }
    }
}

/// Encode one ABI v1 records frame (host → plugin).
pub fn encode_records_frame(
    token: u64,
    generation: u64,
    records: &[VendorRecord],
) -> Result<Vec<u8>, String> {
    if records.len() > MAX_VENDOR_RECORDS {
        return Err("vendor.too_many_records".to_string());
    }
    let mut frame = Vec::with_capacity(28 + records.len().saturating_mul(64));
    frame.extend_from_slice(&RECORDS_MAGIC.to_le_bytes());
    frame.extend_from_slice(&VENDOR_ABI_VERSION.to_le_bytes());
    frame.extend_from_slice(&token.to_le_bytes());
    frame.extend_from_slice(&generation.to_le_bytes());
    frame.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for record in records {
        let class = record.class_name.as_bytes();
        let class_len = u16::try_from(class.len()).map_err(|_| "vendor.class_name_too_long")?;
        frame.extend_from_slice(&class_len.to_le_bytes());
        frame.extend_from_slice(class);
        frame.extend_from_slice(&record.dwg_version.to_le_bytes());
        frame.push(record.source.wire_code());
        frame.extend_from_slice(&record.handle.to_le_bytes());
        push_stream(&mut frame, &record.main, record.main_bits)?;
        push_stream(&mut frame, &record.text, record.text_bits)?;
        push_stream(&mut frame, &record.handle_stream, record.handle_bits)?;
        frame.extend_from_slice(&record.main_skip_bits.to_le_bytes());
        frame.extend_from_slice(&record.text_skip_bits.to_le_bytes());
        frame.extend_from_slice(&record.handle_skip_bits.to_le_bytes());
        frame.extend_from_slice(&(record.references.len() as u32).to_le_bytes());
        for handle in &record.references {
            frame.extend_from_slice(&handle.to_le_bytes());
        }
        if frame.len() > MAX_RECORDS_FRAME_BYTES {
            return Err("vendor.records_frame_too_large".to_string());
        }
    }
    Ok(frame)
}

fn push_stream(frame: &mut Vec<u8>, bytes: &[u8], bits: u32) -> Result<(), String> {
    if bytes.len() > MAX_VENDOR_STREAM_BYTES {
        return Err("vendor.stream_too_long".to_string());
    }
    let length = u32::try_from(bytes.len()).map_err(|_| "vendor.stream_too_long")?;
    frame.extend_from_slice(&bits.to_le_bytes());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(bytes);
    Ok(())
}

/// The revision used by an empty snapshot (no pass).
pub const EMPTY_REVISION: u64 = 0;

/// Convenience: an `Arc`-wrapped empty snapshot.
pub fn empty_store() -> Arc<VendorRepresentationStore> {
    Arc::new(VendorRepresentationStore::empty(EMPTY_REVISION))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{VendorRecord, VendorSource};

    fn record(handle: u64) -> VendorRecord {
        VendorRecord {
            class_name: "TCH_WALL".to_string(),
            handle,
            dwg_version: 1027,
            source: VendorSource::UnknownEntity,
            main: vec![0x06],
            main_bits: 8,
            text: Vec::new(),
            text_bits: 0,
            handle_stream: Vec::new(),
            handle_bits: 0,
            main_skip_bits: 0,
            text_skip_bits: 0,
            handle_skip_bits: 0,
            references: Vec::new(),
        }
    }

    #[test]
    fn the_store_is_keyed_by_source_handle_and_keeps_the_pass_revision() {
        let store = VendorRepresentationStore::new(
            7,
            vec![VendorEntityPrimitives {
                handle: 0x2c,
                primitives: vec![VendorPrimitive::Lines(vec![
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                ])],
            }],
        );
        assert_eq!(store.revision(), 7);
        assert_eq!(store.primitive_count(), 1);
        assert_eq!(store.for_source_handle("2C").map(|p| p.len()), Some(1));
        assert_eq!(store.for_source_handle("0x2c").map(|p| p.len()), Some(1));
        assert_eq!(store.for_source_handle("2D"), None);
    }

    #[test]
    #[ignore]
    fn write_real_fixture_records_frame() {
        let Ok(path) = std::env::var("YACR_TEST_DWG") else {
            return;
        };
        use cad_import_acadrust::Importer as _;
        let bytes = std::fs::read(path).expect("read fixture");
        let request = cad_import_acadrust::ImportRequest {
            document: cad_domain::DocumentId(1),
            database: cad_domain::DatabaseId(1),
            bytes: std::sync::Arc::from(bytes),
            limits: cad_import_acadrust::ImportLimits::default(),
            generation: 0,
            xref: None,
        };
        let drawing = cad_import_acadrust::AcadrustImporter::new()
            .import(&request, &|| false)
            .expect("import");
        let frame = encode_records_frame(1, 1, &drawing.vendor_records).expect("frame");
        let candidates = [
            std::path::PathBuf::from("/tmp/yacr-vendor-frame.bin"),
            std::env::temp_dir().join("yacr-vendor-frame.bin"),
        ];
        let written = candidates
            .iter()
            .find(|candidate| std::fs::write(candidate, &frame).is_ok());
        eprintln!(
            "vendor records={} frame_bytes={} written={:?}",
            drawing.vendor_records.len(),
            frame.len(),
            written
        );
    }

    #[test]
    fn a_records_frame_is_the_stable_v3_header() {
        let frame = encode_records_frame(3, 3, &[record(0x2c)]).expect("encodes");
        assert_eq!(&frame[0..4], &RECORDS_MAGIC.to_le_bytes());
        assert_eq!(&frame[4..8], &VENDOR_ABI_VERSION.to_le_bytes());
        assert_eq!(&frame[8..16], &3u64.to_le_bytes());
        assert_eq!(&frame[16..24], &3u64.to_le_bytes());
        assert_eq!(&frame[24..28], &1u32.to_le_bytes());
    }
}
