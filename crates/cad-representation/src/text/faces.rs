use cad_domain::{CadError, CadResult};
use std::io::Read;
use std::sync::Arc;

use crate::shx::ShxFont;

/// A parsed font face.
pub(super) enum FaceData {
    /// Raw sfnt bytes (TTF/OTF, or WOFF1 already decoded to sfnt).
    Sfnt(Arc<[u8]>),
    /// Compiled AutoCAD shape font.
    Shx(ShxFont),
    /// A face we recognised but cannot decode yet; reported explicitly.
    Unsupported(String),
}

/// Parse bytes into a face, detecting SHX, sfnt and WOFF.
pub(super) fn parse_face(key: &str, bytes: &[u8], encoding: Option<&str>) -> CadResult<FaceData> {
    if bytes.starts_with(b"AutoCAD-") {
        return match ShxFont::parse(bytes, encoding) {
            Ok(font) => Ok(FaceData::Shx(font)),
            // Recognised but undecodable: keep the face so the fallback chain
            // can substitute it, instead of failing registration outright.
            Err(CadError::Unsupported(reason)) => Ok(FaceData::Unsupported(reason)),
            Err(other) => Err(other),
        };
    }
    if bytes.len() >= 4 && &bytes[0..4] == b"wOF2" {
        return Ok(FaceData::Unsupported(
            "WOFF2 fonts are not supported yet".into(),
        ));
    }
    let data = prepare_font(bytes)?;
    ttf_parser::Face::parse(&data, 0)
        .map_err(|e| CadError::CorruptData(format!("font '{key}' cannot be parsed: {e}")))?;
    Ok(FaceData::Sfnt(data))
}

pub(super) fn normalize_key(raw: &str) -> String {
    let bare = raw.trim().trim_matches('"');
    let bare = bare.rsplit(['/', '\\']).next().unwrap_or(bare);
    bare.to_ascii_lowercase()
}

pub(super) fn stem_of(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .filter(|stem| !stem.is_empty())
}

/// Decode WOFF1 to sfnt; pass raw sfnt through.
fn prepare_font(bytes: &[u8]) -> CadResult<Arc<[u8]>> {
    if bytes.len() >= 4 && &bytes[0..4] == b"wOFF" {
        woff_to_sfnt(bytes).map(Arc::from)
    } else {
        Ok(Arc::from(bytes.to_vec()))
    }
}

/// Reconstruct an sfnt (TTF) container from a WOFF1 file.
fn woff_to_sfnt(data: &[u8]) -> CadResult<Vec<u8>> {
    if data.len() < 44 {
        return Err(CadError::CorruptData("WOFF file is truncated".into()));
    }
    let u32be = |o: usize| u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let u16be = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]);

    let flavor = u32be(4);
    let num_tables = u16be(12) as usize;
    let dir_start = 44;
    if num_tables == 0 || dir_start + num_tables * 20 > data.len() {
        return Err(CadError::CorruptData(
            "WOFF table directory is invalid".into(),
        ));
    }

    struct Entry {
        tag: [u8; 4],
        offset: usize,
        comp: usize,
        orig: usize,
        checksum: u32,
    }
    let mut entries = Vec::with_capacity(num_tables);
    for index in 0..num_tables {
        let o = dir_start + index * 20;
        entries.push(Entry {
            tag: [data[o], data[o + 1], data[o + 2], data[o + 3]],
            offset: u32be(o + 4) as usize,
            comp: u32be(o + 8) as usize,
            orig: u32be(o + 12) as usize,
            checksum: u32be(o + 16),
        });
    }
    // sfnt requires the directory sorted by tag.
    entries.sort_by_key(|entry| entry.tag);

    let mut max_power = 0usize;
    while (1usize << (max_power + 1)) <= num_tables {
        max_power += 1;
    }
    let search_range = (1usize << max_power) * 16;
    let entry_selector = max_power;
    let range_shift = num_tables * 16 - search_range;

    let mut header = Vec::with_capacity(12);
    header.extend_from_slice(&flavor.to_be_bytes());
    header.extend_from_slice(&(num_tables as u16).to_be_bytes());
    header.extend_from_slice(&(search_range as u16).to_be_bytes());
    header.extend_from_slice(&(entry_selector as u16).to_be_bytes());
    header.extend_from_slice(&(range_shift as u16).to_be_bytes());

    let mut records: Vec<([u8; 4], u32, u32, u32)> = Vec::with_capacity(num_tables);
    let mut body: Vec<u8> = Vec::new();
    let mut next_offset = 12 + num_tables * 16;
    for entry in &entries {
        if entry.offset + entry.comp > data.len() {
            return Err(CadError::CorruptData("WOFF table extends past EOF".into()));
        }
        let raw = &data[entry.offset..entry.offset + entry.comp];
        let table: Vec<u8> = if entry.comp == entry.orig {
            raw.to_vec()
        } else {
            let mut decoder = flate2::read::ZlibDecoder::new(raw);
            let mut decoded = Vec::with_capacity(entry.orig);
            decoder
                .read_to_end(&mut decoded)
                .map_err(|e| CadError::CorruptData(format!("WOFF table decompress failed: {e}")))?;
            if decoded.len() != entry.orig {
                return Err(CadError::CorruptData(
                    "WOFF table decompressed to the wrong size".into(),
                ));
            }
            decoded
        };
        while next_offset % 4 != 0 {
            next_offset += 1;
            body.push(0);
        }
        records.push((
            entry.tag,
            entry.checksum,
            next_offset as u32,
            entry.orig as u32,
        ));
        body.extend_from_slice(&table);
        next_offset += table.len();
    }

    let mut out = header;
    for (tag, checksum, offset, length) in records {
        out.extend_from_slice(&tag);
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&length.to_be_bytes());
    }
    out.extend_from_slice(&body);
    Ok(out)
}
