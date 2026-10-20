//! Reproducible regeneration of the committed synthetic DWG fixtures.
//!
//! `fixtures/dwg/synthetic-four-lines.dwg` was authored with acadrust's
//! `DwgWriter`, whose `DimStyle::standard()` defaults are *not* a real
//! drawing's: it mixes an 0.18-scale `DIMTXT`/`DIMASZ` with 0.625/1.25/3.75
//! scale gaps and offsets. Against the fixture's ~1 drawing unit of geometry
//! that made dimension annotations unjudgeable in a real-display run:
//!
//! * text floated ~0.805 units above the dimension line (`DIMGAP + DIMTXT +
//!   half an em`), ~160 px at the evidence camera;
//! * extension lines began 0.625 units (`DIMEXO`) above the line — 5.6x the
//!   actual origin-to-line depth — so they were drawn detached;
//! * `DIMDLI = 3.75` units (~745 px) pushed `DIMBASELINE` rows off a 341 px
//!   canvas.
//!
//! `regenerate_synthetic_four_lines` re-reads the committed fixture, rewrites
//! **only** the `Standard` DIMSTYLE to the unit-consistent [`TARGET`] values,
//! and writes the DWG back. The four LINE entities are asserted to keep their
//! exact endpoints; regeneration refuses to touch a file whose geometry
//! differs. These values are a synthetic fixture correction, not a
//! compatibility claim about any vendor drawing.
//!
//! The ignored test is idempotent: after writing it re-reads its own output
//! until the DWG reader/writer pair reaches a fixed point, so a second run
//! leaves the file byte-for-byte alone (and reports "already up to date").
//! The first pass through acadrust's DWG round trip has two incidental,
//! content-neutral effects that are *not* DIMSTYLE edits: the `CLASSES` table
//! is emitted in hash-map order, and an obsolete TABLESTYLE row-suppression
//! flag (DXF code 71) that the DWG object writer does not persist is dropped.
//! Both are stable afterwards, so the committed file is a fixed point.
//!
//! Run it with:
//!
//! ```text
//! cargo test -p cad-import-acadrust --test fixtures_generate -- --ignored
//! ```

use std::io::Cursor;
use std::path::{Path, PathBuf};

use acadrust::{CadDocument, DwgReader, DwgWriter, EntityType};

/// `(DXF variable, value)` pairs written into the `Standard` DIMSTYLE.
///
/// `DIMSCALE` stays 1.0, and `DIMDEC 2`, `DIMZIN 8`, `DIMPOST "<>"`, `DIMTAD 1`
/// are kept as the writer produced them.
const TARGET: &[(&str, f64)] = &[
    // DIMTXT: text height, 0.18 units (~36 px at ~199 px/unit).
    ("dimtxt", 0.18),
    // DIMASZ: arrowhead size, matched to the text.
    ("dimasz", 0.18),
    // DIMGAP: gap between the dimension line and the text.
    ("dimgap", 0.06),
    // DIMEXO: offset from the measured origin to the extension-line start.
    ("dimexo", 0.06),
    // DIMEXE: how far the extension line runs past the dimension line.
    ("dimexe", 0.12),
    // DIMDLI: baseline row spacing; 0.5 units (~100 px) keeps three stacked
    // rows inside a 341 px canvas.
    ("dimdli", 0.5),
];

/// The four LINE endpoints in the committed fixture. Regeneration must not
/// change them.
const EXPECTED_LINES: &[[f64; 4]] = &[
    [0.0, 0.0, 0.0, 1.0],
    [1.0, 0.0, 1.0, 1.0],
    [2.0, 0.0, 2.0, 1.0],
    [3.0, 0.0, 3.0, 1.0],
];

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("dwg")
        .join("synthetic-four-lines.dwg")
}

fn lines(document: &CadDocument) -> Vec<[f64; 4]> {
    document
        .entities()
        .filter_map(|entity| match entity {
            EntityType::Line(line) => Some([line.start.x, line.start.y, line.end.x, line.end.y]),
            _ => None,
        })
        .collect()
}

fn read_document(path: &Path) -> CadDocument {
    DwgReader::from_file(path)
        .unwrap_or_else(|error| panic!("open {}: {error}", path.display()))
        .read()
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn read_document_bytes(bytes: &[u8]) -> CadDocument {
    DwgReader::from_stream(Cursor::new(bytes))
        .read()
        .expect("read in-memory DWG")
}

/// acadrust names the DIMSTYLE fields after the DXF variables, lower-cased:
/// `DIMTXT -> dimtxt`, `DIMASZ -> dimasz`, `DIMGAP -> dimgap`,
/// `DIMEXO -> dimexo`, `DIMEXE -> dimexe`, `DIMDLI -> dimdli`.
fn apply_target_dimstyle(document: &mut CadDocument) {
    let style = document
        .dim_styles
        .get_mut("Standard")
        .expect("fixture has a Standard DIMSTYLE");
    for (field, value) in TARGET {
        match *field {
            "dimtxt" => style.dimtxt = *value,
            "dimasz" => style.dimasz = *value,
            "dimgap" => style.dimgap = *value,
            "dimexo" => style.dimexo = *value,
            "dimexe" => style.dimexe = *value,
            "dimdli" => style.dimdli = *value,
            other => panic!("unmapped DIMSTYLE field `{other}`"),
        }
    }
    assert_eq!(style.name, "Standard");
    assert_eq!(style.dimscale, 1.0, "DIMSCALE stays 1.0");
    assert_eq!(style.dimdec, 2);
    assert_eq!(style.dimzin, 8);
    assert_eq!(style.dimpost, "<>");
    assert_eq!(style.dimtad, 1);
}

fn value_of(style: &acadrust::tables::DimStyle, field: &str) -> f64 {
    match field {
        "dimtxt" => style.dimtxt,
        "dimasz" => style.dimasz,
        "dimgap" => style.dimgap,
        "dimexo" => style.dimexo,
        "dimexe" => style.dimexe,
        "dimdli" => style.dimdli,
        other => panic!("unmapped DIMSTYLE field `{other}`"),
    }
}

/// Asserts the DIMSTYLE corrections and the untouched geometry survived a read.
fn assert_rewritten(document: &CadDocument) {
    assert_eq!(
        lines(document),
        EXPECTED_LINES,
        "geometry must survive regeneration unchanged"
    );
    let style = document
        .dim_styles
        .get("Standard")
        .expect("Standard DIMSTYLE");
    for (field, value) in TARGET {
        assert_eq!(value_of(style, field), *value, "{field} did not round-trip");
    }
    assert_eq!(style.dimscale, 1.0);
    assert_eq!(style.dimdec, 2);
    assert_eq!(style.dimzin, 8);
    assert_eq!(style.dimpost, "<>");
    assert_eq!(style.dimtad, 1);
}

#[test]
#[ignore = "rewrites a committed fixture; run explicitly, never in the default suite"]
fn regenerate_synthetic_four_lines() {
    let path = fixture_path();
    let original = std::fs::read(&path).expect("fixture readable");

    let mut document = read_document(&path);
    assert_eq!(
        lines(&document),
        EXPECTED_LINES,
        "fixture geometry changed; regeneration refuses to overwrite it"
    );

    let style = document
        .dim_styles
        .get("Standard")
        .expect("Standard DIMSTYLE");
    println!(
        "before: DIMTXT={} DIMASZ={} DIMGAP={} DIMEXO={} DIMEXE={} DIMDLI={} DIMSCALE={}",
        style.dimtxt,
        style.dimasz,
        style.dimgap,
        style.dimexo,
        style.dimexe,
        style.dimdli,
        style.dimscale
    );

    apply_target_dimstyle(&mut document);
    assert_eq!(lines(&document), EXPECTED_LINES, "geometry must not change");

    // Let the acadrust reader/writer pair settle: the first hop through the
    // DWG representation reorders `CLASSES` and drops a TABLESTYLE flag that
    // the writer does not persist. Re-reading until bytes stop changing keeps
    // this generator idempotent without masking any content drift (every
    // intermediate document is re-checked for geometry and DIMSTYLE).
    let mut bytes = DwgWriter::write_to_vec(&document).expect("write synthetic DWG");
    for pass in 0..8 {
        let reread = read_document_bytes(&bytes);
        assert_rewritten(&reread);
        let next = DwgWriter::write_to_vec(&reread).expect("rewrite synthetic DWG");
        if next == bytes {
            break;
        }
        assert!(pass < 7, "DWG writer did not reach a fixed point");
        bytes = next;
    }
    assert_rewritten(&read_document_bytes(&bytes));

    if bytes == original {
        println!("fixture already up to date: {}", path.display());
    } else {
        std::fs::write(&path, &bytes).expect("write fixture");
        println!("rewrote {} ({})", path.display(), sha256(&bytes));
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
