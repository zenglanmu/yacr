//! Conservative format sniffing shared by the import dispatcher.
//!
//! This never parses a drawing and never relaxes a format's own validation. It
//! answers exactly one question — which [`Importer`](crate::Importer) should be
//! handed the bytes — and answers `Unknown` rather than guessing.
//!
//! The DWG/DXF arms are byte-for-byte the predicates the acadrust adapter used
//! before this module existed (`looks_like_dwg` plus the ASCII/binary DXF
//! section test). They are copied, not "improved": format detection changing
//! would silently change which files reach the DWG reader.

/// One supported on-disk drawing format.
///
/// `Step` and `Ifc` share the ISO 10303-21 container. They are kept distinct
/// because the STEP adapter must never be handed IFC bytes: IFC has its own
/// entity schema and a later importer owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingFormat {
    Dwg,
    Dxf,
    Step,
    Ifc,
    Obj,
    Stl,
    Ply,
    Off,
    /// An OLE/CFB container: Revit project/project template (`.rvt`/`.rte`) or
    /// family/family template (`.rfa`/`.rft`). The RVT adapter tells them
    /// apart from their streams and refuses families by name.
    Rvt,
    /// A SketchUp model (`.skp`), in either container the reader models: the
    /// MFC `CArchive` stream of releases 3–8 and 2013–2020, or the ZIP/tagged
    /// record container of 2026 and 2027. The SKP adapter refuses a release it
    /// cannot place, and refuses edge-only files by name.
    Skp,
    Unknown,
}

impl DrawingFormat {
    /// Stable human-readable name used in refusal messages.
    pub fn label(self) -> &'static str {
        match self {
            DrawingFormat::Dwg => "DWG",
            DrawingFormat::Dxf => "DXF",
            DrawingFormat::Step => "STEP (ISO 10303-21)",
            DrawingFormat::Ifc => "IFC (ISO 10303-21)",
            DrawingFormat::Obj => "Wavefront OBJ",
            DrawingFormat::Stl => "STL",
            DrawingFormat::Ply => "PLY (Polygon File Format)",
            DrawingFormat::Off => "OFF (Object File Format)",
            DrawingFormat::Rvt => "Autodesk Revit",
            DrawingFormat::Skp => "SketchUp (.skp)",
            DrawingFormat::Unknown => "unknown",
        }
    }
}

/// Identify the format of an untrusted byte stream.
///
/// Order matters only for readability: a DWG signature can never be a DXF
/// section stream, and neither starts with an ISO 10303-21 exchange header.
///
/// The SKP probe runs first because it is a fixed 32-byte signature — cheaper
/// than the OLE/CFB and `AC10xx` sniffs — and no other format starts with
/// `FF FE FF`, so putting it first cannot shadow an arm.
pub fn detect_format(bytes: &[u8]) -> DrawingFormat {
    if looks_like_skp(bytes) {
        DrawingFormat::Skp
    } else if looks_like_ole(bytes) {
        DrawingFormat::Rvt
    } else if looks_like_dwg(bytes) {
        DrawingFormat::Dwg
    } else if looks_like_dxf(bytes) {
        DrawingFormat::Dxf
    } else if looks_like_step(bytes) {
        if ifc_schema(bytes).is_some() {
            DrawingFormat::Ifc
        } else {
            DrawingFormat::Step
        }
    } else if looks_like_stl(bytes) {
        DrawingFormat::Stl
    } else if looks_like_ply(bytes) {
        DrawingFormat::Ply
    } else if looks_like_off(bytes) {
        DrawingFormat::Off
    } else if looks_like_obj(bytes) {
        DrawingFormat::Obj
    } else {
        DrawingFormat::Unknown
    }
}

/// The IFC schema named by an ISO 10303-21 `FILE_SCHEMA` record, if any.
///
/// IFC files are also `ISO-10303-21;`, so the exchange header alone cannot tell
/// them from AP203/AP214/AP242. The `FILE_SCHEMA` record is the discriminator;
/// only the three IFC schema families (`IFC2X3`, `IFC4`, `IFC4X3`) and their
/// `_ADDn`/`_FINAL` extensions are recognised. The scan is bounded because it
/// runs before the file-size limit.
pub fn ifc_schema(bytes: &[u8]) -> Option<String> {
    let record = step_file_schema(bytes)?;
    let upper = record.to_ascii_uppercase();
    const IFC_SCHEMAS: &[&str] = &["IFC2X3", "IFC4", "IFC4X3"];
    IFC_SCHEMAS
        .iter()
        .any(|schema| upper.contains(schema))
        .then(|| record.trim().to_string())
}

/// The raw `FILE_SCHEMA` record text of an ISO 10303-21 byte stream.
///
/// Returns `None` when the header record is missing or outside the bounded
/// sniff window. This is a header read, not a Part 21 parse.
pub fn step_file_schema(bytes: &[u8]) -> Option<String> {
    const SNIFF_BYTES: usize = 256 * 1024;
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(SNIFF_BYTES)]);
    let upper = prefix.to_ascii_uppercase();
    let at = upper.find("FILE_SCHEMA")?;
    let tail = &prefix[at..];
    let end = tail.find(';')?;
    Some(tail[..end].to_string())
}

/// ISO 10303-21 requires the exchange header to be the first record. BOMs and
/// leading whitespace are tolerated; no other prefix is accepted.
fn looks_like_step(bytes: &[u8]) -> bool {
    const SNIFF_BYTES: usize = 256;
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(SNIFF_BYTES)]);
    prefix
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("ISO-10303-21;")
}

/// The fixed 32-byte `.skp` header: a `FF FE FF <len:u8>` UTF-16LE string
/// record whose text is always the document type `SketchUp Model`.
///
/// The `.skp` header is two such string records (document type, then version),
/// then a 16-byte per-model GUID. Only the first record is the signature: it is
/// 14 code units, so the marker plus the text is exactly 32 bytes. The version
/// record is deliberately **not** part of the probe — which release a file is
/// (and whether it is `Verified` or `Unverified`) is the adapter's answer, not
/// detection's. Both containers (the MFC stream and the ZIP/tagged-record one)
/// start with this same record.
fn looks_like_skp(bytes: &[u8]) -> bool {
    const DOCUMENT_TYPE: &str = "SketchUp Model";
    const HEADER_LEN: usize = 4 + DOCUMENT_TYPE.len() * 2;
    if bytes.len() < HEADER_LEN {
        return false;
    }
    if bytes[..4] != [0xFF, 0xFE, 0xFF, DOCUMENT_TYPE.len() as u8] {
        return false;
    }
    DOCUMENT_TYPE
        .encode_utf16()
        .enumerate()
        .all(|(index, unit)| {
            let at = 4 + index * 2;
            bytes[at..at + 2] == unit.to_le_bytes()
        })
}

/// The OLE/CFB magic that every `.rvt`, `.rte`, `.rfa` and `.rft` starts with.
///
/// It is deliberately the container signature only: a family is refused by the
/// RVT adapter (`import.rvt.family_no_geometry`), not by detection, because the
/// importer sees bytes and not a file extension.
fn looks_like_ole(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
}

fn looks_like_dwg(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && &bytes[0..2] == b"AC" && bytes[2..6].iter().all(|b| b.is_ascii_digit())
}

fn looks_like_dxf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"AutoCAD Binary DXF\r\n\x1a\0") || {
        let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
        let mut lines = prefix.lines().map(str::trim);
        matches!((lines.next(), lines.next()), (Some("0"), Some("SECTION")))
    }
}

/// Wavefront OBJ directives that may legally open a file.
///
/// Facets (`f`) are included because a file may legally start with them; the
/// rest are the object/geometry/group/material declarations OBJ producers emit
/// before geometry.
const OBJ_OPENING_DIRECTIVES: &[&str] = &["v", "vt", "vn", "o", "g", "s", "usemtl", "mtllib", "f"];

/// STL has no magic. A binary file is recognised by its standard layout (80-byte
/// header + little-endian facet count whose `84 + 50*count` length matches);
/// ASCII STL must begin with the exact `solid ` prefix the reader probes for.
fn looks_like_stl(bytes: &[u8]) -> bool {
    if bytes.len() >= 84 {
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]);
        let expected = 84u64.saturating_add(u64::from(count).saturating_mul(50));
        if count > 0 && expected == bytes.len() as u64 {
            return true;
        }
    }
    String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).starts_with("solid ")
}

/// PLY has a single-line magic (`ply`) with no other header requirement this
/// sniffer can afford to check. The first line must be exactly `ply`; anything
/// else is left to the OFF/OBJ probes or `Unknown`.
fn looks_like_ply(bytes: &[u8]) -> bool {
    const SNIFF_BYTES: usize = 64;
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(SNIFF_BYTES)]);
    prefix
        .trim_start_matches('\u{feff}')
        .lines()
        .next()
        .map(str::trim)
        == Some("ply")
}

/// The OFF-family prefixes this repository knows by name.
///
/// `OFF` and `COFF` are read; the rest are recognised here only so the OFF
/// adapter can refuse them by name (`import.off.variant_refused`) instead of
/// letting them fall through to the generic unsupported-input message. Detection
/// is an exact first-token match, never a suffix guess.
pub const OFF_FAMILY_PREFIXES: &[&str] = &[
    "OFF", "COFF", "NOFF", "CNOFF", "STOFF", "4OFF", "4NOFF", "4COFF",
];

fn looks_like_off(bytes: &[u8]) -> bool {
    const SNIFF_BYTES: usize = 4096;
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(SNIFF_BYTES)]);
    for raw in prefix.lines().take(32) {
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let token = line.split_whitespace().next().unwrap_or("");
        return OFF_FAMILY_PREFIXES.contains(&token);
    }
    false
}

fn looks_like_obj(bytes: &[u8]) -> bool {
    // Sniff only a bounded prefix: this runs before the file-size limit is
    // applied, so it must not scan an arbitrary blob. Comments and blank lines
    // are skipped, exactly as a reader would.
    const SNIFF_BYTES: usize = 64 * 1024;
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(SNIFF_BYTES)]);
    for raw in prefix.lines() {
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let directive = line.split_whitespace().next().unwrap_or("");
        return OBJ_OPENING_DIRECTIVES.contains(&directive);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dwg_signature_is_dwg() {
        assert_eq!(detect_format(b"AC1027\x00\x00"), DrawingFormat::Dwg);
    }

    #[test]
    fn ascii_dxf_section_stream_is_dxf() {
        assert_eq!(
            detect_format(b"0\nSECTION\n2\nHEADER\n0\nENDSEC\n0\nEOF\n"),
            DrawingFormat::Dxf
        );
    }

    #[test]
    fn binary_dxf_signature_is_dxf() {
        assert_eq!(
            detect_format(b"AutoCAD Binary DXF\r\n\x1a\0rest"),
            DrawingFormat::Dxf
        );
    }

    #[test]
    fn obj_comments_and_blank_lines_are_skipped() {
        assert_eq!(
            detect_format(b"# exported by a tool\n\nv 0 0 0\nf 1 1 1\n"),
            DrawingFormat::Obj
        );
    }

    #[test]
    fn obj_accepts_every_documented_opening_directive() {
        for directive in OBJ_OPENING_DIRECTIVES {
            let bytes = format!("{directive} something\n");
            assert_eq!(
                detect_format(bytes.as_bytes()),
                DrawingFormat::Obj,
                "directive {directive} must open an OBJ file"
            );
        }
    }

    #[test]
    fn ascii_stl_header_is_stl() {
        assert_eq!(
            detect_format(b"solid cube\nfacet normal 0 0 1\nouter loop\n"),
            DrawingFormat::Stl
        );
    }

    #[test]
    fn binary_stl_layout_is_stl() {
        // 80-byte header + count=1 + one 50-byte facet.
        let mut bytes = vec![0u8; 84];
        bytes[80..84].copy_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 50]);
        assert_eq!(detect_format(&bytes), DrawingFormat::Stl);
    }

    /// The fixed `.skp` header: the document type record, a version record and
    /// the 16-byte model GUID (the signature is only the first record).
    fn skp_header(document_type: &str) -> Vec<u8> {
        fn record(text: &str) -> Vec<u8> {
            let mut bytes = vec![0xFF, 0xFE, 0xFF, text.encode_utf16().count() as u8];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
        let mut bytes = record(document_type);
        bytes.extend_from_slice(&record("{17.3.116}"));
        bytes.extend_from_slice(&[0u8; 16]);
        bytes
    }

    #[test]
    fn sketchup_header_is_skp() {
        assert_eq!(
            detect_format(&skp_header("SketchUp Model")),
            DrawingFormat::Skp
        );
        // The signature is the record itself, so trailing container bytes do
        // not change the answer.
        assert_eq!(
            detect_format(&[skp_header("SketchUp Model"), vec![0u8; 64]].concat()),
            DrawingFormat::Skp
        );
    }

    #[test]
    fn a_sketchup_header_with_a_wrong_document_type_is_not_skp() {
        assert_eq!(
            detect_format(&skp_header("SketchUp LayOut")),
            DrawingFormat::Unknown
        );
        assert_eq!(
            detect_format(&skp_header("Sketchup Model")),
            DrawingFormat::Unknown
        );
    }

    #[test]
    fn a_truncated_sketchup_header_is_not_skp() {
        let bytes = skp_header("SketchUp Model");
        // 31 bytes: the marker and 13½ code units are not the signature.
        assert_eq!(detect_format(&bytes[..31]), DrawingFormat::Unknown);
        // The first four bytes alone are not enough either.
        assert_eq!(detect_format(&bytes[..4]), DrawingFormat::Unknown);
    }

    #[test]
    fn ole_magic_is_the_revit_container() {
        assert_eq!(
            detect_format(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1rest"),
            DrawingFormat::Rvt
        );
        // Seven bytes are not enough, and a near-miss is not OLE.
        assert_eq!(detect_format(b"\xD0\xCF\x11\xE0"), DrawingFormat::Unknown);
        assert_eq!(
            detect_format(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE2"),
            DrawingFormat::Unknown
        );
        // glTF is an internal intermediate, never a user-visible format.
        assert_eq!(
            detect_format(b"glTF\x02\x00\x00\x00"),
            DrawingFormat::Unknown
        );
    }

    #[test]
    fn unknown_text_is_not_obj() {
        assert_eq!(detect_format(b"hello world\n"), DrawingFormat::Unknown);
        assert_eq!(detect_format(b""), DrawingFormat::Unknown);
        assert_eq!(detect_format(b"\xde\xad\xbe\xef"), DrawingFormat::Unknown);
        assert_eq!(detect_format(b"OFFICE\n"), DrawingFormat::Unknown);
        assert_eq!(detect_format(b"plywood\n"), DrawingFormat::Unknown);
    }

    #[test]
    fn ply_magic_is_ply_in_every_encoding() {
        assert_eq!(
            detect_format(b"ply\nformat ascii 1.0\nelement vertex 0\nend_header\n"),
            DrawingFormat::Ply
        );
        assert_eq!(
            detect_format(b"ply\r\nformat binary_little_endian 1.0\r\nend_header\r\n"),
            DrawingFormat::Ply
        );
        assert_eq!(
            detect_format(b"ply\nformat binary_big_endian 1.0\nelement vertex 0\nend_header\n"),
            DrawingFormat::Ply
        );
    }

    #[test]
    fn off_and_coff_are_off() {
        assert_eq!(
            detect_format(b"# a comment\n\nOFF\n4 2 0\n"),
            DrawingFormat::Off
        );
        assert_eq!(detect_format(b"COFF\n4 2 0\n"), DrawingFormat::Off);
    }

    #[test]
    fn known_off_variants_are_routed_to_the_off_adapter_for_explicit_refusal() {
        for variant in OFF_FAMILY_PREFIXES {
            let bytes = format!("{variant}\n1 0 0\n");
            assert_eq!(
                detect_format(bytes.as_bytes()),
                DrawingFormat::Off,
                "variant {variant} must reach the OFF adapter, not Unknown"
            );
        }
    }

    #[test]
    fn iso_10303_21_exchange_header_is_step() {
        let ap214 = b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\n";
        assert_eq!(detect_format(ap214), DrawingFormat::Step);
        let ap242 = b"\xef\xbb\xbf  ISO-10303-21;\r\nHEADER;\r\nFILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\r\n";
        assert_eq!(detect_format(ap242), DrawingFormat::Step);
    }

    #[test]
    fn ifc_file_schema_is_ifc_not_step() {
        for schema in ["IFC2X3", "IFC4", "IFC4X3", "IFC4X3_ADD2", "IFC4X1"] {
            let bytes = format!("ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('{schema}'));\nENDSEC;\n");
            assert_eq!(
                detect_format(bytes.as_bytes()),
                DrawingFormat::Ifc,
                "schema {schema} must be refused as IFC, not routed to STEP"
            );
        }
    }

    #[test]
    fn file_schema_text_is_returned_verbatim() {
        let bytes = b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\n";
        assert_eq!(
            step_file_schema(bytes).as_deref(),
            Some("FILE_SCHEMA(('AUTOMOTIVE_DESIGN'))")
        );
        assert_eq!(ifc_schema(bytes), None);
    }
}
