//! Real `HATCH` export (ADR 0018 C/G.2-G.4): byte-level group assertions, the
//! import → export → import round trip and the fixture round trips.
//!
//! Group assertions parse our own writer's output. Writer + our reader agreement
//! is **internal consistency**, not third-party compatibility.

use super::*;
use cad_domain::hatch::GradientStop;
use cad_domain::{
    HatchBoundaryEdge, HatchFill, HatchGeometry, HatchGradient, HatchLoop, HatchLoopFlags,
    HatchPattern, HatchPatternLine, HatchPlane, HatchStyle,
};

// ---------------------------------------------------------------------------
// DXF pair helpers (the writer emits ASCII group-code/value pairs).
// ---------------------------------------------------------------------------

fn pairs_of(bytes: &[u8]) -> Vec<(String, String)> {
    let text = std::str::from_utf8(bytes).expect("ASCII DXF is valid UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    let mut pairs = Vec::new();
    let mut index = 0;
    while index + 1 < lines.len() {
        pairs.push((
            lines[index].trim().to_string(),
            lines[index + 1].trim().to_string(),
        ));
        index += 2;
    }
    pairs
}

/// The group pairs of every `HATCH` record, in entity order.
fn hatch_records(bytes: &[u8]) -> Vec<Vec<(String, String)>> {
    let pairs = pairs_of(bytes);
    let mut records = Vec::new();
    let mut index = 0;
    while index < pairs.len() {
        if pairs[index] == ("0".to_string(), "HATCH".to_string()) {
            let mut record = Vec::new();
            index += 1;
            while index < pairs.len() && pairs[index].0 != "0" {
                record.push(pairs[index].clone());
                index += 1;
            }
            records.push(record);
        } else {
            index += 1;
        }
    }
    records
}

fn hatch_record(bytes: &[u8]) -> Vec<(String, String)> {
    hatch_records(bytes)
        .into_iter()
        .next()
        .expect("the export wrote a HATCH record")
}

fn values(record: &[(String, String)], code: &str) -> Vec<String> {
    record
        .iter()
        .filter(|(group, _)| group == code)
        .map(|(_, value)| value.clone())
        .collect()
}

fn single(record: &[(String, String)], code: &str) -> String {
    let found = values(record, code);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one group {code}, got {found:?}"
    );
    found[0].clone()
}

fn number(record: &[(String, String)], code: &str) -> f64 {
    single(record, code)
        .parse()
        .unwrap_or_else(|_| panic!("group {code} is not a number"))
}

fn integer(record: &[(String, String)], code: &str) -> i64 {
    single(record, code)
        .parse()
        .unwrap_or_else(|_| panic!("group {code} is not an integer"))
}

// ---------------------------------------------------------------------------
// Domain hatch builders.
// ---------------------------------------------------------------------------

const EXTERNAL: u32 = HatchLoopFlags::EXTERNAL;
const DERIVED: u32 = HatchLoopFlags::DERIVED;
const POLYLINE: u32 = HatchLoopFlags::POLYLINE;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, flags: u32) -> HatchLoop {
    HatchLoop {
        points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        bulges: vec![0.0; 4],
        flags: HatchLoopFlags(flags),
        source_edges: None,
    }
}

/// The four analytic line edges of a rectangle (the shape the importer keeps for
/// an edge-form path).
fn rect_edges(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<HatchBoundaryEdge> {
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    (0..4)
        .map(|index| HatchBoundaryEdge::Line {
            start: corners[index],
            end: corners[(index + 1) % 4],
        })
        .collect()
}

fn with_edges(mut loop_: HatchLoop, x0: f64, y0: f64, x1: f64, y1: f64) -> HatchLoop {
    loop_.source_edges = Some(rect_edges(x0, y0, x1, y1));
    loop_
}

struct Spec {
    loops: Vec<HatchLoop>,
    style: HatchStyle,
    fill: HatchFill,
    seeds: Vec<[f64; 2]>,
    pixel_size: Option<f64>,
    pattern_type: cad_domain::HatchPatternType,
    elevation: f64,
}

impl Default for Spec {
    fn default() -> Self {
        Spec {
            loops: vec![rect(0.0, 0.0, 10.0, 10.0, EXTERNAL)],
            style: HatchStyle::Normal,
            fill: HatchFill::Solid,
            seeds: Vec::new(),
            pixel_size: None,
            pattern_type: cad_domain::HatchPatternType::Predefined,
            elevation: 0.0,
        }
    }
}

/// Build a hatch through the import constructor so no creation-path rejection
/// interferes with the export contract (styles, raw bits, seeds are all free).
fn build(spec: Spec) -> SemanticGeometry {
    let (hatch, _) = HatchGeometry::from_import(
        spec.loops,
        HatchPlane::new(p(0.0, 0.0, 1.0), spec.elevation),
        spec.style,
        spec.fill,
        spec.seeds,
        spec.pixel_size,
        spec.pattern_type,
    );
    SemanticGeometry::Hatch(Box::new(hatch))
}

fn pattern_fill(name: &str, angle: f64, scale: f64, double: bool) -> HatchFill {
    HatchFill::Pattern(HatchPattern {
        name: name.to_string(),
        angle,
        scale,
        double,
        lines: vec![HatchPatternLine {
            angle: 0.25,
            base_x: 0.5,
            base_y: 0.25,
            offset_x: 0.0,
            offset_y: 1.5,
            dashes: vec![0.5, -0.25],
        }],
    })
}

fn gradient_fill() -> HatchFill {
    HatchFill::Gradient(HatchGradient::TwoColor {
        name: "LINEAR".into(),
        reserved: 7,
        angle_raw: 0.75,
        shift: 0.25,
        stops: vec![
            GradientStop {
                value: 0.0,
                rgb: [255, 0, 0],
            },
            GradientStop {
                value: 1.0,
                rgb: [0, 0, 255],
            },
        ],
    })
}

// ---------------------------------------------------------------------------
// Byte-level group assertions.
// ---------------------------------------------------------------------------

#[test]
fn a_solid_hatch_is_written_as_a_real_hatch_with_71_zero_and_no_47() {
    let (bytes, report) = export_one(build(Spec::default()));
    assert_eq!(
        report.counts.converted, 1,
        "a hatch export always declares its conversions"
    );
    assert!(matches!(read_entities(&bytes)[0], EntityType::Hatch(_)));

    let record = hatch_record(&bytes);
    // Groups that identify the record.
    assert_eq!(single(&record, "2"), "SOLID");
    assert_eq!(integer(&record, "70"), 1, "solid fill");
    // ADR 0018 §0.4: associativity is not modelled, so 71 is always 0.
    assert_eq!(integer(&record, "71"), 0);
    assert_eq!(integer(&record, "91"), 1, "one boundary path");
    assert_eq!(
        integer(&record, "97"),
        0,
        "no boundary handles are ever written"
    );
    assert_eq!(integer(&record, "75"), 0, "Normal");
    assert_eq!(integer(&record, "76"), 1, "Predefined");
    // Elevation origin and normal.
    assert_eq!(number(&record, "30"), 0.0);
    assert_eq!(number(&record, "230"), 1.0);
    // No derived loop, so no pixel size (the writer couples the two).
    assert!(values(&record, "47").is_empty(), "47 must be absent");
    // No gradient block.
    assert!(values(&record, "450").is_empty());
}

#[test]
fn a_loop_without_source_edges_uses_the_polyline_path_form() {
    let (bytes, report) = export_one(build(Spec::default()));
    let record = hatch_record(&bytes);
    // 92 carries the raw external bit plus the polyline bit the path form needs.
    assert_eq!(integer(&record, "92"), (EXTERNAL | POLYLINE) as i64);
    assert_eq!(integer(&record, "72"), 0, "no bulges in this loop");
    assert_eq!(integer(&record, "73"), 1, "loops are implicitly closed");
    assert_eq!(integer(&record, "93"), 4);
    assert_eq!(values(&record, "10").len(), 5, "elevation + four vertices");
    assert!(values(&record, "42").is_empty(), "no 42 without a bulge");
    assert!(report.entries.iter().any(|entry| {
        entry.code == "export.converted.hatch" && entry.message.contains("no analytic source edges")
    }));
}

#[test]
fn a_loop_with_source_edges_uses_the_edge_form() {
    let spec = Spec {
        loops: vec![with_edges(
            rect(0.0, 0.0, 10.0, 10.0, EXTERNAL),
            0.0,
            0.0,
            10.0,
            10.0,
        )],
        ..Spec::default()
    };
    let (bytes, report) = export_one(build(spec));
    let record = hatch_record(&bytes);
    assert_eq!(integer(&record, "92"), EXTERNAL as i64, "no polyline bit");
    assert_eq!(integer(&record, "93"), 4, "four analytic edges");
    assert_eq!(values(&record, "72"), vec!["1"; 4], "line edges");
    // A line edge has no floating-point payload: only arcs/splines add 73/74.
    assert!(values(&record, "73").is_empty());
    assert_eq!(
        values(&record, "10").len(),
        5,
        "elevation + four start points"
    );
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.message.contains("sampled polyline")),
        "an analytic boundary is not a degradation"
    );
}

#[test]
fn group_47_appears_exactly_when_a_loop_is_derived() {
    let derived = Spec {
        loops: vec![rect(0.0, 0.0, 10.0, 10.0, EXTERNAL | DERIVED)],
        pixel_size: Some(0.25),
        ..Spec::default()
    };
    let (bytes, report) = export_one(build(derived));
    let record = hatch_record(&bytes);
    assert_eq!(
        integer(&record, "92"),
        (EXTERNAL | DERIVED | POLYLINE) as i64
    );
    assert_eq!(number(&record, "47"), 0.25);
    assert!(report
        .entries
        .iter()
        .any(|entry| entry.message.contains("group 47")));

    // A derived loop without a stored pixel size still writes 47 (the writer's
    // rule), as 0.0, and the report says the value was absent.
    let no_size = Spec {
        loops: vec![rect(0.0, 0.0, 10.0, 10.0, DERIVED)],
        ..Spec::default()
    };
    let (bytes, report) = export_one(build(no_size));
    assert_eq!(number(&hatch_record(&bytes), "47"), 0.0);
    assert!(report
        .entries
        .iter()
        .any(|entry| entry.message.contains("stored no pixel size")));

    // Without a derived loop the stored value is not written at all.
    let unused = Spec {
        pixel_size: Some(0.5),
        ..Spec::default()
    };
    let (bytes, report) = export_one(build(unused));
    assert!(values(&hatch_record(&bytes), "47").is_empty());
    assert!(report
        .entries
        .iter()
        .any(|entry| entry.message.contains("is not written")));
}

#[test]
fn a_pattern_hatch_writes_its_style_type_angle_scale_double_and_definition_rows() {
    let angle = 30.0_f64.to_radians();
    let spec = Spec {
        fill: pattern_fill("CUSTOM_RIDGE", angle, 2.5, true),
        style: HatchStyle::Outer,
        pattern_type: cad_domain::HatchPatternType::Custom,
        ..Spec::default()
    };
    let (bytes, report) = export_one(build(spec));
    let record = hatch_record(&bytes);

    assert_eq!(single(&record, "2"), "CUSTOM_RIDGE");
    assert_eq!(integer(&record, "70"), 0, "pattern fill, not solid");
    assert_eq!(
        integer(&record, "71"),
        0,
        "associativity is never preserved"
    );
    assert_eq!(integer(&record, "75"), 1, "Outer");
    assert_eq!(integer(&record, "76"), 2, "Custom");
    // The writer converts group 52 to degrees itself, so the domain keeps
    // radians and the file keeps the writer's convention.
    assert!((number(&record, "52") - 30.0).abs() < 1e-9);
    assert!((number(&record, "41") - 2.5).abs() < 1e-9);
    assert_eq!(integer(&record, "77"), 1, "double");
    assert_eq!(integer(&record, "78"), 1, "one definition row");
    assert!((number(&record, "53") - 0.25_f64.to_degrees()).abs() < 1e-9);
    assert!((number(&record, "43") - 0.5).abs() < 1e-12);
    assert!((number(&record, "44") - 0.25).abs() < 1e-12);
    assert!((number(&record, "45") - 0.0).abs() < 1e-12);
    assert!((number(&record, "46") - 1.5).abs() < 1e-12);
    assert_eq!(integer(&record, "79"), 2, "two dashes");
    assert_eq!(values(&record, "49"), vec!["0.5", "-0.25"]);
    // No clipped lines: the only 10/20 pairs are the elevation, the boundary
    // vertices and (zero) seeds.
    assert_eq!(values(&record, "98"), vec!["0"]);
    assert!(report
        .entries
        .iter()
        .any(|entry| entry.code == "export.converted.hatch"));
}

/// An in-plane rotation about world Z, applied to a hatch after import.
fn rotation_z(theta: f64) -> Transform3 {
    let (sin, cos) = theta.sin_cos();
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = cos;
    matrix[0][1] = -sin;
    matrix[1][0] = sin;
    matrix[1][1] = cos;
    Transform3 { matrix }
}

#[test]
fn a_rotated_pattern_writes_the_raw_rows_and_the_rotated_hatch_angle() {
    // ADR U-1/U-7: the writer emits the *definition*, never `families()` output.
    // A hatch rotated after import keeps its raw definition rows and records the
    // rotation in group 52 only. Writing `families()` instead would bake the
    // angle and scale into the rows and, with `77=1`, double the row count.
    let angle = 10.0_f64.to_radians();
    let spec = Spec {
        fill: pattern_fill("CUSTOM_RIDGE", angle, 2.5, true),
        pattern_type: cad_domain::HatchPatternType::Custom,
        ..Spec::default()
    };
    let SemanticGeometry::Hatch(hatch) = build(spec) else {
        panic!("expected a hatch geometry");
    };
    let rotated = hatch
        .transformed(&rotation_z(25.0_f64.to_radians()))
        .expect("an in-plane rotation is a similarity");
    let (bytes, report) = export_one(SemanticGeometry::Hatch(Box::new(rotated)));
    let record = hatch_record(&bytes);

    // Group 52 carries the rotated hatch-level angle; group 41 is untouched.
    assert!((number(&record, "52") - (10.0 + 25.0)).abs() < 1e-9);
    assert!((number(&record, "41") - 2.5).abs() < 1e-9);
    // One raw row, not the two families a `double` pattern composes to.
    assert_eq!(integer(&record, "78"), 1, "raw definition rows only");
    assert_eq!(integer(&record, "77"), 1, "the raw double flag");
    // The row itself is the imported definition, not a rotated/scaled copy.
    assert!((number(&record, "53") - 0.25_f64.to_degrees()).abs() < 1e-9);
    assert!((number(&record, "43") - 0.5).abs() < 1e-12);
    assert!((number(&record, "44") - 0.25).abs() < 1e-12);
    assert!((number(&record, "45") - 0.0).abs() < 1e-12);
    assert!((number(&record, "46") - 1.5).abs() < 1e-12);
    assert_eq!(values(&record, "49"), vec!["0.5", "-0.25"]);
    assert!(report.entries.iter().any(|entry| {
        entry.code == "export.converted.hatch" && entry.message.contains("1 definition row")
    }));
}

#[test]
fn seeds_are_written_after_the_98_count() {
    let spec = Spec {
        seeds: vec![[1.5, 2.5], [3.0, 4.0]],
        ..Spec::default()
    };
    let (bytes, _) = export_one(build(spec));
    let record = hatch_record(&bytes);
    assert_eq!(integer(&record, "98"), 2);
    let foot = record
        .iter()
        .position(|(group, value)| group == "98" && value == "2")
        .expect("seed count");
    let tail: Vec<(String, String)> = record[foot + 1..].to_vec();
    assert_eq!(
        tail,
        vec![
            ("10".to_string(), "1.5".to_string()),
            ("20".to_string(), "2.5".to_string()),
            ("10".to_string(), "3.0".to_string()),
            ("20".to_string(), "4.0".to_string()),
        ],
        "the seed points follow the 98 count"
    );
}

#[test]
fn the_gradient_block_is_written_exactly_for_a_gradient_fill() {
    let (solid_bytes, _) = export_one(build(Spec::default()));
    let solid = hatch_record(&solid_bytes);
    for code in [
        "450", "451", "460", "461", "452", "462", "453", "463", "470",
    ] {
        assert!(
            values(&solid, code).is_empty(),
            "a solid fill must not write group {code}"
        );
    }

    let spec = Spec {
        fill: gradient_fill(),
        ..Spec::default()
    };
    let (bytes, report) = export_one(build(spec));
    let record = hatch_record(&bytes);
    assert_eq!(
        integer(&record, "71"),
        0,
        "associativity is never preserved"
    );
    assert_eq!(integer(&record, "70"), 1, "a gradient is a solid fill");
    assert_eq!(integer(&record, "450"), 1);
    assert_eq!(integer(&record, "451"), 7, "reserved value round-trips");
    assert!(
        (number(&record, "460") - 0.75).abs() < 1e-12,
        "opaque angle"
    );
    assert!((number(&record, "461") - 0.25).abs() < 1e-12);
    assert_eq!(integer(&record, "452"), 0, "two-colour gradient");
    assert_eq!(integer(&record, "453"), 2);
    assert_eq!(values(&record, "463"), vec!["0.0", "1.0"]);
    assert_eq!(values(&record, "470"), vec!["LINEAR"]);
    // 63 + 421 per stop: the RGB payload survives, the names cannot.
    assert_eq!(values(&record, "63").len(), 2);
    assert_eq!(values(&record, "421").len(), 2);
    assert!(report.entries.iter().any(|entry| {
        entry.code == "export.converted.hatch" && entry.message.contains("colour names")
    }));
}

// ---------------------------------------------------------------------------
// Import -> export -> import round trips.
// ---------------------------------------------------------------------------

fn single_hatch(bytes: &[u8]) -> HatchGeometry {
    let database = import_bytes(bytes);
    let found = database
        .entities()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Hatch(hatch) => Some(hatch.as_ref().clone()),
            _ => None,
        })
        .expect("the export re-imports as a HATCH");
    found
}

#[test]
fn a_pattern_hatch_round_trips_style_type_seeds_bits_and_rows() {
    let spec = Spec {
        loops: vec![
            with_edges(rect(0.0, 0.0, 10.0, 10.0, EXTERNAL), 0.0, 0.0, 10.0, 10.0),
            with_edges(rect(2.0, 2.0, 8.0, 8.0, 0), 2.0, 2.0, 8.0, 8.0),
        ],
        style: HatchStyle::Outer,
        fill: pattern_fill("CUSTOM_RIDGE", 0.5, 2.5, true),
        seeds: vec![[1.5, 2.5], [3.0, 4.0]],
        pattern_type: cad_domain::HatchPatternType::Custom,
        elevation: 2.0,
        ..Spec::default()
    };
    let (bytes, _) = export_one(build(spec));
    let after = single_hatch(&bytes);

    assert_eq!(after.style(), HatchStyle::Outer);
    assert_eq!(after.pattern_type(), cad_domain::HatchPatternType::Custom);
    assert_eq!(after.seeds(), &[[1.5, 2.5], [3.0, 4.0]]);
    assert_eq!(after.loops().len(), 2);
    assert_eq!(after.loops()[0].flags.bits(), EXTERNAL, "raw 92 bits");
    assert_eq!(after.loops()[1].flags.bits(), 0);
    assert!((after.plane().elevation - 2.0).abs() < 1e-9);
    let HatchFill::Pattern(pattern) = after.fill() else {
        panic!("expected a pattern fill, got {:?}", after.fill());
    };
    assert_eq!(pattern.name, "CUSTOM_RIDGE");
    assert!((pattern.angle - 0.5).abs() < 1e-12);
    assert!((pattern.scale - 2.5).abs() < 1e-12);
    assert!(pattern.double);
    assert_eq!(pattern.lines.len(), 1);
    let row = &pattern.lines[0];
    assert!((row.angle - 0.25).abs() < 1e-12);
    assert!((row.base_x - 0.5).abs() < 1e-12);
    assert!((row.base_y - 0.25).abs() < 1e-12);
    assert!((row.offset_x - 0.0).abs() < 1e-12);
    assert!((row.offset_y - 1.5).abs() < 1e-12);
    assert_eq!(row.dashes, vec![0.5, -0.25]);
    // The analytic edges are still analytic (the export did not resample them).
    assert_eq!(
        after.loops()[0].source_edges.as_ref().map(Vec::len),
        Some(4)
    );
}

#[test]
fn a_gradient_hatch_round_trips_its_name_reserved_angle_and_stops() {
    let spec = Spec {
        fill: gradient_fill(),
        ..Spec::default()
    };
    let (bytes, _) = export_one(build(spec));
    let after = single_hatch(&bytes);
    let HatchFill::Gradient(gradient) = after.fill() else {
        panic!("expected a gradient fill, got {:?}", after.fill());
    };
    match gradient {
        HatchGradient::TwoColor {
            name,
            reserved,
            angle_raw,
            shift,
            stops,
        } => {
            assert_eq!(name, "LINEAR");
            assert_eq!(*reserved, 7);
            assert!((*angle_raw - 0.75).abs() < 1e-12);
            assert!((*shift - 0.25).abs() < 1e-12);
            assert_eq!(stops.len(), 2);
            assert!((stops[0].value - 0.0).abs() < 1e-12);
            assert_eq!(stops[0].rgb, [255, 0, 0]);
            assert_eq!(stops[1].rgb, [0, 0, 255]);
        }
        other => panic!("expected a two-colour gradient, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Fixture round trips (ADR G.3).
// ---------------------------------------------------------------------------

fn fixture_bytes(path: &str) -> Arc<[u8]> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join(path)).expect("fixture readable");
    Arc::from(bytes.into_boxed_slice())
}

fn all_hatches(database: &DrawingDatabase) -> Vec<&HatchGeometry> {
    database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Hatch(hatch) => Some(hatch.as_ref()),
            SemanticGeometry::Compound(_) => panic!("a hatch imported/exported as a Compound"),
            _ => None,
        })
        .collect()
}

fn sorted_loop_counts(database: &DrawingDatabase) -> Vec<usize> {
    let mut counts: Vec<usize> = all_hatches(database)
        .iter()
        .map(|hatch| hatch.loops().len())
        .collect();
    counts.sort_unstable();
    counts
}

fn assert_hatch_type_keys(database: &DrawingDatabase) {
    for entity in database.entities() {
        if matches!(entity.geometry, SemanticGeometry::Hatch(_)) {
            assert_eq!(entity.object.type_key, "AcDbHatch");
        }
    }
}

#[test]
fn flange_hatches_survive_the_export_as_hatches() {
    let imported = import_bytes(&fixture_bytes("fixtures/dxf/qcad-flange/flange.dxf"));
    let before = sorted_loop_counts(&imported);
    assert_eq!(before, vec![1, 5, 6, 7, 7, 8]);
    assert_hatch_type_keys(&imported);
    assert_eq!(all_hatches(&imported).len(), 6, "all six stay first-class");

    // Five of the six flange hatches live in the `Title` block, which is only
    // referenced from paper-space layouts; v1 exports model space, so only the
    // model-space hatch is written and the unreferenced block is declared.
    let (bytes, report) = export_db(&imported);
    assert_eq!(hatch_records(&bytes).len(), 1);
    assert!(report
        .entries
        .iter()
        .any(|entry| entry.code == "export.dropped.unreferenced_block"));
    let reimported = import_bytes(&bytes);
    assert_eq!(sorted_loop_counts(&reimported), vec![1]);
    assert_hatch_type_keys(&reimported);

    // The model-space hatch is the `76=0` one carrying `78=1`: it keeps its
    // definition row, name, scale and pattern type.
    let user_defined: Vec<&HatchGeometry> = all_hatches(&reimported)
        .into_iter()
        .filter(|hatch| hatch.pattern_type() == cad_domain::HatchPatternType::UserDefined)
        .collect();
    assert_eq!(user_defined.len(), 1);
    let HatchFill::Pattern(pattern) = user_defined[0].fill() else {
        panic!("expected the user-defined hatch to stay a pattern");
    };
    assert_eq!(pattern.name, "ANSI31");
    assert_eq!(pattern.lines.len(), 1, "78=1 is written back");
    assert!((pattern.scale - 0.5).abs() < 1e-12);
}

#[test]
fn isometric_grid_solid_hatch_survives_the_export() {
    let imported = import_bytes(&fixture_bytes(
        "fixtures/dxf/qcad-examples/isometric_grid.dxf",
    ));
    let before = sorted_loop_counts(&imported);
    assert_eq!(before, vec![1, 9]);

    let (bytes, _) = export_db(&imported);
    assert_eq!(hatch_records(&bytes).len(), 2);
    let reimported = import_bytes(&bytes);
    assert_eq!(sorted_loop_counts(&reimported), before);
    assert_hatch_type_keys(&reimported);
    // The nine-loop SOLID is a pattern-free solid with no definition rows.
    let nine = all_hatches(&reimported)
        .into_iter()
        .find(|hatch| hatch.loops().len() == 9)
        .expect("the nine-loop hatch");
    assert!(matches!(nine.fill(), HatchFill::Solid));
}

/// ADR G.4/N5: every committed QCAD fixture is style `75=0`, so the `Outer` and
/// `Ignore` styles can only be exercised by a synthetic drawing. This fixture
/// carries a three-level nested set twice (one `Outer`, one `Ignore`) and two
/// disjoint outer loops.
#[test]
fn the_multi_style_fixture_imports_fills_and_round_trips() {
    let imported = import_bytes(&fixture_bytes("fixtures/dxf/hatch-styles/multi-style.dxf"));
    let hatches = all_hatches(&imported);
    assert_eq!(hatches.len(), 3);
    assert_hatch_type_keys(&imported);

    let mut styles: Vec<HatchStyle> = hatches.iter().map(|hatch| hatch.style()).collect();
    styles.sort_by_key(|style| format!("{style:?}"));
    assert_eq!(
        styles,
        vec![HatchStyle::Ignore, HatchStyle::Normal, HatchStyle::Outer]
    );

    // The nested set is three levels deep; the disjoint hatch has two loops.
    let mut counts: Vec<usize> = hatches.iter().map(|hatch| hatch.loops().len()).collect();
    counts.sort_unstable();
    assert_eq!(counts, vec![2, 3, 3]);

    // Style semantics on the real file: Normal 64 + 4, Outer 64, Ignore 100.
    let area_by_style = |style: HatchStyle| -> f64 {
        let hatch = hatches
            .iter()
            .find(|hatch| hatch.style() == style)
            .expect("style present");
        display_area(hatch)
    };
    assert!((area_by_style(HatchStyle::Outer) - 6400.0).abs() < 1e-6);
    assert!((area_by_style(HatchStyle::Ignore) - 10_000.0).abs() < 1e-6);
    assert!((area_by_style(HatchStyle::Normal) - 200.0).abs() < 1e-6);

    let (bytes, _) = export_db(&imported);
    assert_eq!(hatch_records(&bytes).len(), 3);
    let reimported = import_bytes(&bytes);
    let mut after: Vec<HatchStyle> = all_hatches(&reimported)
        .iter()
        .map(|hatch| hatch.style())
        .collect();
    after.sort_by_key(|style| format!("{style:?}"));
    assert_eq!(after, styles, "75 round-trips");
    assert_eq!(sorted_loop_counts(&reimported), counts);
    assert_hatch_type_keys(&reimported);
}

fn display_area(hatch: &HatchGeometry) -> f64 {
    hatch
        .display()
        .iter()
        .map(|geometry| match geometry {
            SemanticGeometry::Mesh(mesh) => mesh
                .triangles
                .iter()
                .map(|triangle| {
                    let a = mesh.vertices[triangle[0] as usize];
                    let b = mesh.vertices[triangle[1] as usize];
                    let c = mesh.vertices[triangle[2] as usize];
                    let ab = [b.x - a.x, b.y - a.y, b.z - a.z];
                    let ac = [c.x - a.x, c.y - a.y, c.z - a.z];
                    let cross = [
                        ab[1] * ac[2] - ab[2] * ac[1],
                        ab[2] * ac[0] - ab[0] * ac[2],
                        ab[0] * ac[1] - ab[1] * ac[0],
                    ];
                    (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() / 2.0
                })
                .sum(),
            _ => 0.0,
        })
        .sum()
}
