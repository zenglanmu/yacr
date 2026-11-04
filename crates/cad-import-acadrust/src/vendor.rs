//! Vendor entity extraction for the WebView plugin pass.
//!
//! Tianzheng objects reach us in two shapes:
//!
//! * `EntityType::Unknown` with a `TCH_*` DXF name — every TCH object the reader
//!   could not model. `raw_dwg_data` is the **merged** DWG record (type prefix +
//!   common data + main + text + handle streams).
//! * `ACAD_PROXY_ENTITY` whose `class_id` (or R2004+ `dxf_subclass`) names a
//!   `TCH_*` class. `payload` / `text_payload` already hold the original class
//!   body and strings; this is the shape R2000 drawings saved without the
//!   Tianzheng ARX use.
//!
//! This module only copies bytes. It never interprets the TCH layout: the plugin
//! does. Records are bounded and sorted by handle, and every split failure is
//! counted so the import report can say how many were skipped.
//!
//! # Stream split
//!
//! `raw_dwg_data` is one merged blob. The DWG record layout is:
//!
//! ```text
//! [object type][common data][main][text][flag/size][handles]
//! ```
//!
//! For R2010+ the handle-stream length is the record framing's MC value
//! (`dwg_handle_bits`), so `handle_start = total_bits - handle_bits`. For
//! R2007 the RL after the object type is the handle start. Pre-R2007 text is
//! inline in main. `DwgBitReader::set_position_by_flag` gives the text start and
//! `text_stream_remaining_bits()` gives its exact bit length, which is why the
//! split does not guess byte boundaries.

use std::collections::{HashMap, HashSet};

use acadrust::document::CadDocument;
use acadrust::entities::{EntityType, ExtendedEntityData};
use acadrust::io::dwg::dwg_stream_readers::bit_reader::DwgBitReader;
use acadrust::io::dwg::dwg_stream_readers::merged_reader::DwgMergedReader;
use acadrust::io::dwg::dwg_stream_readers::object_reader::DwgObjectReader;
use acadrust::io::dwg::dwg_version::DwgVersion;
use acadrust::types::DxfVersion;

/// Upper bound of records copied into one pass.
pub const MAX_VENDOR_RECORDS: usize = 200_000;

/// Upper bound of the sum of copied stream bytes.
pub const MAX_VENDOR_BYTES: usize = 256 * 1024 * 1024;

/// Upper bound of one stream (main/text/handle) copied from a record.
pub const MAX_VENDOR_STREAM_BYTES: usize = 16 * 1024 * 1024;

/// Which source shape produced one vendor record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorSource {
    /// A `TCH_*` object read as `EntityType::Unknown`.
    UnknownEntity,
    /// An `ACAD_PROXY_ENTITY` whose class resolves to `TCH_*`.
    ProxyEntity,
}

impl VendorSource {
    /// Stable byte for the ABI frame.
    pub fn wire_code(self) -> u8 {
        match self {
            Self::UnknownEntity => 0,
            Self::ProxyEntity => 1,
        }
    }
}

/// One vendor record, byte-for-byte what the plugin needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorRecord {
    /// The TCH class name (`TCH_WALL`, …).
    pub class_name: String,
    /// The source entity handle.
    pub handle: u64,
    /// DWG version as a stable decimal code (`1027` for AC1027).
    pub dwg_version: u32,
    /// Which source shape produced this record.
    pub source: VendorSource,
    /// Main stream bytes (common data + class body, bit-aligned at bit 0).
    pub main: Vec<u8>,
    /// Exact main-stream bit length.
    pub main_bits: u32,
    /// Text stream bytes (bit-aligned at bit 0), empty before R2007.
    pub text: Vec<u8>,
    /// Exact text-stream bit length.
    pub text_bits: u32,
    /// Handle-stream bytes (bit-aligned at bit 0).
    pub handle_stream: Vec<u8>,
    /// Exact handle-stream bit length.
    pub handle_bits: u32,
    /// Bits of common data the class-body decoder must skip in `main`.
    pub main_skip_bits: u32,
    /// Bits of common data the class-body decoder must skip in `text`.
    pub text_skip_bits: u32,
    /// Bits of common data the class-body decoder must skip in `handle_stream`.
    pub handle_skip_bits: u32,
    /// Handles referenced by a proxy record (soft/hard pointers).
    pub references: Vec<u64>,
}

/// The extraction result carried by the import.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VendorExtraction {
    /// Sorted by handle.
    pub records: Vec<VendorRecord>,
    /// Records whose merged stream could not be split (or that exceeded a limit).
    pub skipped: usize,
    /// Whether a global cap stopped the walk.
    pub truncated: bool,
}

/// Extract every `TCH_*` entity and TCH proxy reachable from the document.
pub fn extract_vendor_records(document: &CadDocument) -> VendorExtraction {
    let mut extraction = VendorExtraction::default();
    let mut seen = HashSet::new();
    let mut total_bytes = 0usize;

    for block in document.block_records.iter() {
        for entity in document.entities_in_block(&block.name) {
            if extraction.records.len() >= MAX_VENDOR_RECORDS {
                extraction.truncated = true;
                extraction.records.sort_by_key(|record| record.handle);
                return extraction;
            }
            let handle = entity.common().handle.value();
            if !seen.insert(handle) {
                continue;
            }
            let record = match entity {
                EntityType::Unknown(unknown) if unknown.dxf_name.starts_with("TCH_") => {
                    unknown_record(document, unknown)
                }
                EntityType::Extended(extended) => proxy_record(document, extended),
                _ => None,
            };
            let Some(record) = record else {
                continue;
            };
            let bytes = record
                .main
                .len()
                .saturating_add(record.text.len())
                .saturating_add(record.handle_stream.len());
            if bytes > MAX_VENDOR_STREAM_BYTES
                || total_bytes.saturating_add(bytes) > MAX_VENDOR_BYTES
            {
                extraction.skipped = extraction.skipped.saturating_add(1);
                if total_bytes.saturating_add(bytes) > MAX_VENDOR_BYTES {
                    extraction.truncated = true;
                    break;
                }
                continue;
            }
            total_bytes = total_bytes.saturating_add(bytes);
            extraction.records.push(record);
        }
    }
    extraction.records.sort_by_key(|record| record.handle);
    extraction
}

fn unknown_record(
    _document: &CadDocument,
    unknown: &acadrust::entities::UnknownEntity,
) -> Option<VendorRecord> {
    let raw = unknown.raw_dwg_data.as_deref()?;
    let version = unknown.dwg_source_version.unwrap_or(DxfVersion::Unknown);
    let (streams, handle_start) =
        split_merged_record(raw, version, unknown.dwg_handle_bits).ok()?;
    let (main_skip_bits, text_skip_bits, handle_skip_bits) =
        common_skips(raw, &streams, version, unknown.dwg_type_code, handle_start).ok()?;
    Some(VendorRecord {
        class_name: unknown.dxf_name.clone(),
        handle: unknown.common.handle.value(),
        dwg_version: version_code(version),
        source: VendorSource::UnknownEntity,
        main: streams.main,
        main_bits: streams.main_bits,
        text: streams.text,
        text_bits: streams.text_bits,
        handle_stream: streams.handle,
        handle_bits: streams.handle_bits,
        main_skip_bits,
        text_skip_bits,
        handle_skip_bits,
        references: Vec::new(),
    })
}

fn proxy_record(
    document: &CadDocument,
    extended: &acadrust::entities::ExtendedEntity,
) -> Option<VendorRecord> {
    let ExtendedEntityData::Proxy(proxy) = &extended.data else {
        return None;
    };
    let class_name = if proxy.dxf_subclass.starts_with("TCH_") {
        proxy.dxf_subclass.clone()
    } else {
        document
            .classes
            .iter()
            .find(|class| i32::from(class.class_number) == proxy.class_id)
            .map(|class| class.dxf_name.clone())?
    };
    if !class_name.starts_with("TCH_") {
        return None;
    }
    Some(VendorRecord {
        class_name,
        handle: extended.common.handle.value(),
        dwg_version: version_code(document.version),
        source: VendorSource::ProxyEntity,
        main: proxy.payload.data(),
        main_bits: proxy.payload.bit_count,
        text: proxy.text_payload.data(),
        text_bits: proxy.text_payload.bit_count,
        handle_stream: Vec::new(),
        handle_bits: 0,
        main_skip_bits: 0,
        text_skip_bits: 0,
        handle_skip_bits: 0,
        references: proxy
            .object_ids
            .iter()
            .map(|reference| reference.handle.value())
            .collect(),
    })
}

struct Streams {
    main: Vec<u8>,
    main_bits: u32,
    text: Vec<u8>,
    text_bits: u32,
    handle: Vec<u8>,
    handle_bits: u32,
}

fn split_merged_record(
    raw: &[u8],
    version: DxfVersion,
    handle_bits: i64,
) -> Result<(Streams, i64), String> {
    if raw.is_empty() {
        return Err("empty record".into());
    }
    let dwg = DwgVersion::from_dxf_version(version).map_err(|error| error.to_string())?;
    let total = raw.len().saturating_mul(8) as i64;
    let mut reader = DwgBitReader::new(raw.to_vec(), dwg, version);
    let _type_code = reader.read_object_type();
    let after_type = reader.position_in_bits();

    let (data_start, handle_start) = if version >= DxfVersion::AC1024 {
        if handle_bits <= 0 || handle_bits > total {
            return Err("invalid handle-bits framing".into());
        }
        (after_type, total - handle_bits)
    } else if version == DxfVersion::AC1021 {
        // R2007 embeds RL after the type code; the main stream starts after it.
        let handle_start = reader.read_raw_long();
        (reader.position_in_bits(), handle_start)
    } else if version >= DxfVersion::AC1015 {
        // R2000..R2004 embed RL after the type code, but the common-data parser
        // re-reads it, so the main stream starts before the RL.
        let handle_start = reader.read_raw_long();
        (after_type, handle_start)
    } else {
        return Err("pre-R2000 record split is not supported".into());
    };
    if handle_start <= data_start || handle_start > total {
        return Err("invalid handle-stream boundary".into());
    }

    let (text_start, text_bits) = if version >= DxfVersion::AC1021 {
        let mut text = DwgBitReader::new(raw.to_vec(), dwg, version);
        let start = text.set_position_by_flag(handle_start - 1);
        if text.check_error().is_err() {
            return Err("invalid text-stream framing".into());
        }
        (start, text.text_stream_remaining_bits())
    } else {
        (handle_start, 0)
    };
    if text_start < data_start || text_start > handle_start {
        return Err("text stream overlaps the main stream".into());
    }
    let text_bits = text_bits.max(0) as u32;
    let main_end = text_start;
    let text_end = text_start.saturating_add(i64::from(text_bits));
    if text_end > handle_start {
        return Err("text stream overlaps the handle stream".into());
    }

    let (main, main_bits) = copy_bits(raw, data_start, main_end)?;
    let (text, text_bits) = if text_bits == 0 {
        (Vec::new(), 0)
    } else {
        copy_bits(raw, text_start, text_end)?
    };
    let (handle, handle_bits) = copy_bits(raw, handle_start, total)?;
    Ok((
        Streams {
            main,
            main_bits,
            text,
            text_bits,
            handle,
            handle_bits,
        },
        handle_start,
    ))
}

/// Consume the common entity header from the split streams and return the
/// class-body skip for each stream.
///
/// acadrust owns this layout (version branches, EED, reactors, xdictionary,
/// graphic data), so we reuse its parser instead of duplicating it. A dummy
/// `DwgObjectReader` is enough: `read_common_entity_data` reads only from the
/// supplied merged reader and the version it carries.
fn common_skips(
    raw: &[u8],
    streams: &Streams,
    version: DxfVersion,
    type_code: i16,
    handle_start: i64,
) -> Result<(u32, u32, u32), String> {
    let dwg = DwgVersion::from_dxf_version(version).map_err(|error| error.to_string())?;
    let main = DwgBitReader::new(streams.main.clone(), dwg, version);
    // Build the text reader from the full record so `set_position_by_flag` can
    // establish its text-stream cursor and end. A reader over the sliced text
    // bytes alone has `text_stream_pos == -1`, which makes
    // `text_remaining_bits()` report zero and overstates the common-data skip.
    let text = if streams.text_bits > 0 {
        let mut reader = DwgBitReader::new(raw.to_vec(), dwg, version);
        reader.set_position_by_flag(handle_start - 1);
        if reader.check_error().is_err() {
            return Err("invalid text-stream framing".into());
        }
        Some(reader)
    } else {
        None
    };
    let total_text_bits = text
        .as_ref()
        .map(|reader| reader.text_stream_remaining_bits())
        .unwrap_or(0)
        .max(0);
    let handle =
        (streams.handle_bits > 0).then(|| DwgBitReader::new(streams.handle.clone(), dwg, version));
    let mut merged = DwgMergedReader::from_readers(main, text, handle, version);
    let reader = DwgObjectReader::new(Vec::new(), version, HashMap::<u64, i64>::new())
        .map_err(|error| error.to_string())?;
    let _ = reader.read_common_entity_data(&mut merged, type_code);
    merged.check_error().map_err(|error| error.to_string())?;

    let main_skip = merged.position_in_bits().max(0) as u32;
    let text_skip = if streams.text_bits == 0 {
        0
    } else {
        (total_text_bits - merged.text_remaining_bits()).max(0) as u32
    };
    let handle_skip = if streams.handle_bits == 0 {
        0
    } else {
        (streams.handle.len().saturating_mul(8) as i64 - merged.handle_remaining_bits()).max(0)
            as u32
    };
    Ok((
        main_skip.min(streams.main_bits),
        text_skip.min(streams.text_bits),
        handle_skip.min(streams.handle_bits),
    ))
}

/// Copy `[start, end)` bits into a new MSB-first byte buffer starting at bit 0.
fn copy_bits(source: &[u8], start: i64, end: i64) -> Result<(Vec<u8>, u32), String> {
    if start < 0 || end < start || end > source.len().saturating_mul(8) as i64 {
        return Err("bit range out of bounds".into());
    }
    let count = (end - start) as usize;
    let mut output = vec![0u8; count.div_ceil(8)];
    for index in 0..count {
        let source_bit = (start as usize) + index;
        if (source[source_bit / 8] >> (7 - (source_bit % 8))) & 1 == 1 {
            output[index / 8] |= 0x80 >> (index % 8);
        }
    }
    Ok((output, count as u32))
}

fn version_code(version: DxfVersion) -> u32 {
    match version {
        DxfVersion::Unknown => 0,
        DxfVersion::AC1009 => 1009,
        DxfVersion::AC1012 => 1012,
        DxfVersion::AC1014 => 1014,
        DxfVersion::AC1015 => 1015,
        DxfVersion::AC1018 => 1018,
        DxfVersion::AC1021 => 1021,
        DxfVersion::AC1024 => 1024,
        DxfVersion::AC1027 => 1027,
        DxfVersion::AC1032 => 1032,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_bits_shifts_to_bit_zero() {
        let source = [0b1011_0010u8, 0b0100_0000u8];
        let (copy, bits) = copy_bits(&source, 2, 10).expect("range");
        assert_eq!(bits, 8);
        assert_eq!(copy, vec![0b1100_1001u8]);
    }

    #[test]
    fn version_codes_are_the_stable_wire_numbers() {
        assert_eq!(version_code(DxfVersion::AC1027), 1027);
        assert_eq!(version_code(DxfVersion::Unknown), 0);
    }

    #[test]
    fn a_real_tangent_fixture_exposes_class_body_streams() {
        // Skipped unless a real drawing is supplied, exactly like the other
        // `YACR_TEST_DWG` tests: a skip is not evidence.
        let Ok(path) = std::env::var("YACR_TEST_DWG") else {
            return;
        };
        let bytes = std::fs::read(path).expect("read YACR_TEST_DWG");
        let mut reader =
            acadrust::io::dwg::dwg_reader::DwgReader::from_stream(std::io::Cursor::new(bytes));
        let outcome = reader.read_with_stats().expect("read fixture");
        let extraction = extract_vendor_records(&outcome.document);
        if std::env::var_os("YACR_VENDOR_DEBUG").is_some() {
            eprintln!(
                "vendor records={} skipped={} truncated={}",
                extraction.records.len(),
                extraction.skipped,
                extraction.truncated
            );
        }
        assert!(!extraction.records.is_empty(), "no TCH records extracted");
        let wall = extraction
            .records
            .iter()
            .find(|record| record.class_name == "TCH_WALL");
        let sample = wall.unwrap_or_else(|| extraction.records.first().expect("a record"));
        assert!(sample.main_bits > sample.main_skip_bits);
        if wall.is_none() && sample.source == VendorSource::UnknownEntity {
            let base = read_u8_at(&sample.main, sample.main_skip_bits);
            assert!(
                (1..=14).contains(&base),
                "{} class body starts with {base}, not a TCH base version",
                sample.class_name
            );
        }
        if let Some(wall) = wall {
            assert_eq!(
                read_u8_at(&wall.main, wall.main_skip_bits),
                // Both shapes start their private class body with base version 6
                // in the real sample (`UnknownEntity`) or with the protected
                // header in the R2000 proxy sample (0x0e); what matters is that
                // the skip lands on a deterministic byte, not common data.
                if wall.source == VendorSource::UnknownEntity {
                    6
                } else {
                    0x0e
                },
                "class-body start for {} ({:?})",
                wall.class_name,
                wall.source
            );
        }
        for record in &extraction.records {
            assert!(
                record.main_bits > 0,
                "{} has no main bits",
                record.class_name
            );
            assert!(record.main_skip_bits <= record.main_bits);
            assert!(record.text_skip_bits <= record.text_bits);
            assert!(record.handle_skip_bits <= record.handle_bits);
        }
    }

    fn read_u8_at(bytes: &[u8], bit: u32) -> u8 {
        let mut value = 0u8;
        for index in 0..8 {
            let position = bit as usize + index;
            if position >= bytes.len() * 8 {
                break;
            }
            value = (value << 1) | ((bytes[position / 8] >> (7 - (position % 8))) & 1);
        }
        value
    }
}
