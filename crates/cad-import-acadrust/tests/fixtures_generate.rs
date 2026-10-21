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
//! `regenerate_synthetic_acis_cube_dxf` authors the *drawing* counterpart of
//! the raw `fixtures/acis/cube.sat` payload: one `3DSOLID` at the origin on
//! layer 0, written as an ASCII DXF. It exists so the import-time ACIS
//! tessellation can be seen in a running window, not only asserted in unit
//! tests. AC1024 is requested on purpose: from AC1027 the acadrust DXF writer
//! relocates ACIS data into binary SAB chunks in `ACDSDATA`, while pre-AC1027
//! keeps it inline as text lines (group code 1, DXF version-1 character cipher
//! over the SAT stream). The generator refuses to overwrite a file that is not
//! the same one-solid drawing, and validates the bytes through our importer
//! before writing them.
//!
//! Run it with:
//!
//! ```text
//! cargo test -p cad-import-acadrust --test fixtures_generate -- --ignored
//! ```

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use acadrust::entities::Solid3D;
use acadrust::{CadDocument, DwgReader, DwgWriter, DxfReader, DxfVersion, DxfWriter, EntityType};
use cad_domain::{DatabaseId, DocumentId, Mesh, SemanticGeometry};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// Raw ACIS payload the drawing fixture is built from.
const CUBE_SAT: &str = "fixtures/acis/cube.sat";
/// The generated drawing fixture.
const ACIS_CUBE_DXF: &str = "fixtures/dxf/synthetic-acis-cube.dxf";
/// `cube.sat` spans [-1, 1] on every axis; a box tessellates to 12 triangles.
const CUBE_TRIANGLES: usize = 12;
/// Half extent of `cube.sat`, used to give the DXF header real extents instead
/// of the "not computed" sentinel (the round-trip check pins it).
const CUBE_HALF_EXTENT: f64 = 1.0;

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

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative)
}

/// The minimal drawing: one `3DSOLID` built from `cube.sat`, no transform, on
/// layer 0. A cold `CadDocument` already carries the `0` layer that the entity
/// references, so nothing else has to be synthesized.
fn acis_cube_document() -> CadDocument {
    let sat = std::fs::read_to_string(repo_path(CUBE_SAT))
        .unwrap_or_else(|error| panic!("read {CUBE_SAT}: {error}"));
    let mut document = CadDocument::new();
    let mut solid = Solid3D::from_sat(&sat);
    solid.common.layer = "0".to_string();
    document
        .add_entity(EntityType::Solid3D(solid))
        .expect("add the 3DSOLID to the drawing");
    // The header otherwise keeps the 1e20 "extents not computed" sentinel.
    document.header.model_space_extents_min =
        acadrust::Vector3::new(-CUBE_HALF_EXTENT, -CUBE_HALF_EXTENT, -CUBE_HALF_EXTENT);
    document.header.model_space_extents_max =
        acadrust::Vector3::new(CUBE_HALF_EXTENT, CUBE_HALF_EXTENT, CUBE_HALF_EXTENT);
    document
}

/// Writes the drawing as ASCII DXF.
///
/// `AC1024` is deliberate: `needs_sab()` in the acadrust writer switches to
/// binary SAB in `ACDSDATA` at `AC1027`, which would turn the ASCII fixture into
/// an opaque binary blob. Pre-AC1027 keeps the SAT stream as inline text lines
/// (the DXF version-1 character cipher still applies, but it stays ASCII).
fn acis_cube_dxf(document: &CadDocument) -> Vec<u8> {
    DxfWriter::new(document)
        .with_version(DxfVersion::AC1024)
        .write_to_vec()
        .expect("write the synthetic DXF")
}

fn read_dxf_document(bytes: &[u8]) -> CadDocument {
    DxfReader::from_reader(Cursor::new(bytes.to_vec()))
        .expect("open in-memory DXF")
        .read()
        .expect("read in-memory DXF")
}

/// The single-source-of-truth assertion for "this drawing is the fixture".
fn assert_one_cube_solid(document: &CadDocument) {
    let solids: Vec<&Solid3D> = document
        .entities()
        .filter_map(|entity| match entity {
            EntityType::Solid3D(solid) => Some(solid),
            _ => None,
        })
        .collect();
    assert_eq!(solids.len(), 1, "the fixture holds exactly one 3DSOLID");
    assert!(
        !solids[0].acis_data.sat_data.is_empty(),
        "the 3DSOLID must carry SAT text, not an empty or SAB-only payload"
    );
    assert_eq!(solids[0].common.layer, "0", "the solid lives on layer 0");
}

fn mesh_bounds(mesh: &Mesh) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in &mesh.vertices {
        min[0] = min[0].min(point.x);
        min[1] = min[1].min(point.y);
        min[2] = min[2].min(point.z);
        max[0] = max[0].max(point.x);
        max[1] = max[1].max(point.y);
        max[2] = max[2].max(point.z);
    }
    (min, max)
}

/// Round-trips the bytes through *our* importer, which is the property that
/// makes the fixture fit for a real-display check: the `3DSOLID` must arrive as
/// tessellated [`SemanticGeometry::Mesh`] geometry, not as an opaque proxy.
fn assert_imports_as_tessellated_cube(bytes: &[u8]) {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 1,
    };
    let drawing = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("the ACIS drawing fixture must import");

    let meshes: Vec<&Mesh> = drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .collect();
    assert_eq!(
        meshes.len(),
        1,
        "the single 3DSOLID must arrive as exactly one Mesh"
    );

    let mesh = meshes[0];
    assert_eq!(
        mesh.triangles.len(),
        CUBE_TRIANGLES,
        "a box tessellates to {CUBE_TRIANGLES} triangles"
    );
    assert!(
        !mesh.vertices.is_empty(),
        "a tessellated solid must have vertices"
    );

    let (min, max) = mesh_bounds(mesh);
    for (actual, expected) in min.iter().zip([-CUBE_HALF_EXTENT; 3]) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "min bounds {min:?} do not match the cube extent"
        );
    }
    for (actual, expected) in max.iter().zip([CUBE_HALF_EXTENT; 3]) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "max bounds {max:?} do not match the cube extent"
        );
    }
    println!(
        "round trip: {} triangle(s), bounds {min:?}..{max:?}",
        mesh.triangles.len()
    );
}

#[test]
#[ignore = "writes a committed fixture; run explicitly, never in the default suite"]
fn regenerate_synthetic_acis_cube_dxf() {
    let document = acis_cube_document();
    assert_one_cube_solid(&document);

    let bytes = acis_cube_dxf(&document);
    // The reader must agree with the writer before anything reaches disk.
    let reread = read_dxf_document(&bytes);
    assert_one_cube_solid(&reread);
    assert_imports_as_tessellated_cube(&bytes);

    let path = repo_path(ACIS_CUBE_DXF);
    match std::fs::read(&path) {
        Ok(existing) if existing == bytes => {
            println!("fixture already up to date: {}", path.display());
        }
        Ok(existing) => {
            // Refuse to overwrite a file that is not the same one-solid drawing.
            let previous = read_dxf_document(&existing);
            assert_one_cube_solid(&previous);
            std::fs::write(&path, &bytes).expect("write fixture");
            println!("rewrote {} ({})", path.display(), sha256(&bytes));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create fixture directory");
            }
            std::fs::write(&path, &bytes).expect("write fixture");
            println!("wrote {} ({})", path.display(), sha256(&bytes));
        }
        Err(error) => panic!("read {}: {error}", path.display()),
    }
}

#[test]
fn committed_acis_cube_dxf_is_reproducible_and_tessellates() {
    let path = repo_path(ACIS_CUBE_DXF);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "read {}: {error}; run the ignored generator",
            path.display()
        )
    });
    assert_eq!(
        bytes,
        acis_cube_dxf(&acis_cube_document()),
        "{} is stale; run regenerate_synthetic_acis_cube_dxf",
        path.display()
    );
    assert_imports_as_tessellated_cube(&bytes);
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
