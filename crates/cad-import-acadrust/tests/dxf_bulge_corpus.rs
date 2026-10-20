//! Geometric invariants over every non-zero bulge in the committed DXF corpus.
//!
//! `dxf_samples.rs` (in `cad-cli-tools`) asserts that the QCAD corpus imports
//! and builds a representation without failures. That is a *structural* gate: it
//! never looked at the shape of a bulge arc, so a mirrored arc expansion
//! survived in `fixtures/dxf/qcad-flange/flange.dxf` (six non-zero bulges) and
//! `fixtures/dxf/qcad-examples/entities.dxf` (three, one a 254-degree major arc)
//! unnoticed. This test closes that gap:
//!
//! 1. every bulged polyline segment's **sampled apex lies on the side of its
//!    chord opposite the centre**, i.e. at the signed perpendicular offset
//!    `-bulge * chord / 2` (positive bulge = counter-clockwise, so its apex is
//!    to the *right* of the chord);
//! 2. every sample lies on the **analytic circle** of that chord + bulge and
//!    the samples cover the analytic angular span in the analytic direction;
//! 3. the number of checked segments is reported **and pinned per fixture**, so
//!    a silent corpus change (or a segment that starts being skipped) shows up
//!    as a count failure instead of quietly reducing coverage.
//!
//! Skipped segments are counted and reported by reason: a degenerate chord
//! (`< 1e-9`), a bulge below the stated epsilon (`|bulge| < 1e-6`, where the
//! apex offset is `|b|c/2` and f64 noise would make the side test meaningless),
//! and a polyline that is not parallel to world XY. The last one matters
//! because the sampler is exercised on an isolated two-point polyline, which the
//! tessellator reads in the world-XY bulge frame; a tilted source polyline has
//! its bulges in another plane (`cad_geometry::is_xy_parallel` says which).
//!
//! This lives here rather than in `cad-cli-tools` because it needs both the
//! importer (bulge-carrying `SemanticGeometry`) and `cad_geometry`'s
//! tessellator; `cad-cli-tools` declares neither. It is a corpus test, not
//! vendor-compatibility or visual evidence.

#[path = "../../cad-domain/tests/support/bulge_cases.rs"]
mod bulge_cases;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_domain::{Point3, SemanticGeometry};
use cad_geometry::{tessellate_geometry, TessellationParams};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// Segments with `|bulge|` below this are not checked for the apex side: the
/// apex offset is `|bulge| * chord / 2`, and below 1e-6 it is inside f64 noise
/// for ordinary drawing coordinates.
const BULGE_EPS: f64 = 1e-6;

/// Chords shorter than this are treated as degenerate (no arc to speak of).
const CHORD_EPS: f64 = 1e-9;

/// Pinned coverage: `(fixture, checked bulge segments)`. A corpus edit that adds
/// or removes a bulge, or a segment that starts being skipped, fails here.
const PINNED: &[(&str, usize)] = &[
    ("fixtures/dxf/hatch-styles/multi-style.dxf", 0),
    ("fixtures/dxf/qcad-examples/calibration.dxf", 0),
    ("fixtures/dxf/qcad-examples/colors.dxf", 0),
    ("fixtures/dxf/qcad-examples/entities.dxf", 3),
    ("fixtures/dxf/qcad-examples/example00.dxf", 0),
    ("fixtures/dxf/qcad-examples/example01.dxf", 0),
    ("fixtures/dxf/qcad-examples/isometric_grid.dxf", 0),
    ("fixtures/dxf/qcad-examples/linetypes.dxf", 0),
    ("fixtures/dxf/qcad-examples/lineweights.dxf", 0),
    ("fixtures/dxf/qcad-examples/projection.dxf", 0),
    ("fixtures/dxf/qcad-flange/flange.dxf", 6),
];

#[derive(Debug, Default)]
struct Tally {
    checked: usize,
    skipped_degenerate: usize,
    skipped_small_bulge: usize,
    skipped_plane: usize,
}

fn cargo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn collect_dxf(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("readable fixture directory entry").path();
        if path.is_dir() {
            collect_dxf(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "dxf") {
            out.push(path);
        }
    }
}

fn import(path: &Path) -> cad_db::DrawingDatabase {
    let bytes: Arc<[u8]> = Arc::from(
        std::fs::read(path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .into_boxed_slice(),
    );
    let request = ImportRequest {
        document: cad_domain::DocumentId(1),
        database: cad_domain::DatabaseId(1),
        bytes,
        limits: ImportLimits::default(),
        generation: 1,
    };
    AcadrustImporter::new()
        .import(&request, &|| false)
        .unwrap_or_else(|e| panic!("{}: import failed: {e}", path.display()))
        .database
}

fn flat(p: Point3) -> [f64; 2] {
    [p.x, p.y]
}

#[test]
fn committed_dxf_bulges_bow_to_the_side_their_sign_encodes() {
    bulge_cases::assert_table_is_hand_computed();
    let params = TessellationParams::default();
    let mut root = cargo_root();
    root.push("fixtures");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_dxf(&root, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no fixtures found under {}",
        root.display()
    );

    let mut report: Vec<(String, Tally)> = Vec::new();
    for file in &files {
        let relative = file
            .strip_prefix(cargo_root())
            .expect("fixtures live under the crate root")
            .to_string_lossy()
            .replace('\\', "/");
        let database = import(file);
        let mut tally = Tally::default();
        for entity in database.entities() {
            let SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } = &entity.geometry
            else {
                continue;
            };
            if bulges.iter().all(|bulge| *bulge == 0.0) {
                continue;
            }
            let count = points.len();
            let last = if *closed {
                count
            } else {
                count.saturating_sub(1)
            };
            let plane_ok = cad_geometry::is_xy_parallel(points);
            for index in 0..last {
                let bulge = bulges.get(index).copied().unwrap_or(0.0);
                if bulge == 0.0 {
                    continue;
                }
                let a = points[index];
                let b = points[(index + 1) % count];
                let chord = (b.x - a.x).hypot(b.y - a.y);
                if !plane_ok {
                    tally.skipped_plane += 1;
                    continue;
                }
                if !chord.is_finite() || chord < CHORD_EPS {
                    tally.skipped_degenerate += 1;
                    continue;
                }
                if !bulge.is_finite() || bulge.abs() < BULGE_EPS {
                    tally.skipped_small_bulge += 1;
                    continue;
                }
                // Sample exactly this segment: an isolated two-point polyline
                // goes through the same display tessellator the viewer uses.
                let segment = SemanticGeometry::Polyline {
                    points: vec![a, b],
                    bulges: vec![bulge],
                    closed: false,
                };
                let samples: Vec<[f64; 2]> = tessellate_geometry(&segment, params)
                    .iter()
                    .map(|point| flat(*point))
                    .collect();
                let case = bulge_cases::analytic("corpus", flat(a), flat(b), bulge);
                let label = format!(
                    "corpus {relative} entity {} handle {:?} segment {index}",
                    entity.id.0, entity.object.source_handle
                );
                bulge_cases::assert_samples(&label, &case, &samples, params.tolerance + 1e-9);
                bulge_cases::assert_apex(&label, &case, &samples, 2.0 * params.tolerance + 1e-9);
                tally.checked += 1;
            }
        }
        report.push((relative, tally));
    }

    // Report first, so a failure below still shows what was covered.
    let mut total = Tally::default();
    for (file, tally) in &report {
        eprintln!(
            "{file}: checked {} bulge segment(s); skipped {} degenerate chord(s), \
             {} below |bulge| {BULGE_EPS}, {} non-XY-plane",
            tally.checked, tally.skipped_degenerate, tally.skipped_small_bulge, tally.skipped_plane
        );
        total.checked += tally.checked;
        total.skipped_degenerate += tally.skipped_degenerate;
        total.skipped_small_bulge += tally.skipped_small_bulge;
        total.skipped_plane += tally.skipped_plane;
    }
    eprintln!(
        "total: {} bulge segment(s) checked over {} fixture(s); skipped {} degenerate, \
         {} below |bulge| {BULGE_EPS}, {} non-XY-plane",
        total.checked,
        report.len(),
        total.skipped_degenerate,
        total.skipped_small_bulge,
        total.skipped_plane
    );

    // Pinned counts: a silent corpus change shows up as a count change here.
    assert_eq!(
        report.len(),
        PINNED.len(),
        "fixture count changed: {:?}",
        report.iter().map(|(file, _)| file).collect::<Vec<_>>()
    );
    for ((file, tally), (pinned_file, pinned)) in report.iter().zip(PINNED.iter()) {
        assert_eq!(file, pinned_file, "fixture list changed");
        assert_eq!(
            tally.checked, *pinned,
            "{file}: checked {} bulge segment(s), the corpus pin says {pinned}",
            tally.checked
        );
        // The committed corpus is 2D DXF: no segment may be skipped for being
        // out of plane, and none of its bulges is below the epsilon.
        assert_eq!(
            tally.skipped_plane, 0,
            "{file}: {} bulge segment(s) skipped as non-XY-plane",
            tally.skipped_plane
        );
        assert_eq!(
            tally.skipped_small_bulge, 0,
            "{file}: {} bulge segment(s) below the |bulge| epsilon",
            tally.skipped_small_bulge
        );
    }
    assert_eq!(
        total.checked, 9,
        "the corpus must keep covering at least the flange (6) and entities (3) bulges"
    );
}
