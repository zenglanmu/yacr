//! Predefined HATCH pattern fidelity: rows resolved from the built-in table.
//!
//! A `76=1` (predefined) hatch whose file carries no `78` definition rows used
//! to import as a pattern with zero line families, i.e. `Partial` with
//! "hatch pattern has no line families" and no fill. The importer now resolves
//! such a name against `cad_domain::hatch::predefined_pattern` and imports the
//! table's rows with the file's own scale/angle/double, recording
//! `import.hatch_pattern_table` because those rows are **our transcription**
//! (ADR U-1), not vendor-verified values.
//!
//! The three rules pinned here are: the file's own rows always win (no table
//! lookup, no diagnostic), an unimplemented name fabricates nothing (still
//! `Partial`, now naming the pattern), and `76=2` (`Custom`) is never resolved.

use super::*;
use acadrust::entities::BoundaryPath;

const TABLE_CODE: &str = "import.hatch_pattern_table";

fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BoundaryPath {
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut path = BoundaryPath::new();
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
            start: acadrust::types::Vector2::new(a.0, a.1),
            end: acadrust::types::Vector2::new(b.0, b.1),
        }));
    }
    path
}

/// A non-solid hatch with one square boundary and no definition rows.
fn rowless_hatch(name: &str, pattern_type: acadrust::entities::HatchPatternType) -> Hatch {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = false;
    hatch.pattern_type = pattern_type;
    hatch.pattern.name = name.to_string();
    hatch.pattern_angle = 0.0;
    hatch.pattern_scale = 1.0;
    hatch.is_double = false;
    assert!(
        hatch.pattern.lines.is_empty(),
        "the fixture must carry no definition rows"
    );
    hatch.paths = vec![rect_path(0.0, 0.0, 10.0, 8.0)];
    hatch
}

struct Converted {
    geometry: SemanticGeometry,
    completeness: Completeness,
    diagnostics: Vec<Diagnostic>,
}

fn convert(hatch: Hatch) -> Converted {
    let req = request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder =
        ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));
    let entity = EntityType::Hatch(Box::new(hatch));
    let common = entity.common();
    let (geometry, completeness) = builder.convert(&entity, common);
    Converted {
        geometry,
        completeness,
        diagnostics: builder.diagnostics,
    }
}

fn imported_pattern(geometry: &SemanticGeometry) -> &HatchPattern {
    let SemanticGeometry::Hatch(hatch) = geometry else {
        panic!("expected a first-class hatch, got {geometry:?}");
    };
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill, got {:?}", hatch.fill());
    };
    pattern
}

fn partial_reasons(completeness: &Completeness) -> Vec<String> {
    match completeness {
        Completeness::Partial(reasons) => reasons.clone(),
        other => panic!("expected Partial, got {other:?}"),
    }
}

fn table_diagnostics(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == TABLE_CODE)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect()
}

#[test]
fn predefined_pattern_without_rows_imports_the_table_rows_and_keeps_the_file_scalars() {
    let mut hatch = rowless_hatch("ANSI31", acadrust::entities::HatchPatternType::Predefined);
    hatch.pattern_angle = 30.0f64.to_radians();
    hatch.pattern_scale = 2.5;
    hatch.is_double = true;
    let out = convert(hatch);

    let pattern = imported_pattern(&out.geometry);
    // The table's rows, converted the way the table documents (row angle in
    // degrees, hatch-level angle in radians here).
    let table = cad_domain::hatch::predefined_pattern("ANSI31").expect("ANSI31 is implemented");
    assert_eq!(pattern.lines, table.to_hatch_pattern(2.5, 30.0, true).lines);
    assert_eq!(pattern.lines.len(), 1);
    // The file's own scalars survive: nothing is taken from the table but rows.
    assert_eq!(pattern.name, "ANSI31");
    assert!((pattern.scale - 2.5).abs() < 1e-12, "{}", pattern.scale);
    assert!(
        (pattern.angle - 30.0f64.to_radians()).abs() < 1e-12,
        "{}",
        pattern.angle
    );
    assert!(pattern.double);
    // The pattern now composes into real line families, so the hatch is no
    // longer partial for the missing-rows reason.
    assert!(!pattern.families().is_empty());
    assert!(
        matches!(out.completeness, Completeness::Complete),
        "{:?}",
        out.completeness
    );
    // Provenance: our transcription, not the file's definition (ADR U-1).
    let reported = table_diagnostics(&out.diagnostics);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(reported[0].contains("ANSI31"), "{reported:?}");
    assert!(reported[0].contains("ADR U-1"), "{reported:?}");
    assert!(reported[0].contains("not vendor-verified"), "{reported:?}");
}

#[test]
fn predefined_pattern_without_rows_resolves_a_multi_family_table_entry() {
    let mut hatch = rowless_hatch("ANSI37", acadrust::entities::HatchPatternType::Predefined);
    hatch.pattern_scale = 0.5;
    let out = convert(hatch);

    let pattern = imported_pattern(&out.geometry);
    assert_eq!(pattern.lines.len(), 2, "{:?}", pattern.lines);
    let expected = cad_domain::hatch::predefined_pattern("ANSI37").expect("ANSI37 is implemented");
    assert_eq!(
        pattern.lines,
        expected.to_hatch_pattern(0.5, 0.0, false).lines
    );
    // Both crossed families (45 and 135 degrees) reach the composed pattern.
    let families = pattern.families();
    assert_eq!(families.len(), 2, "{families:?}");
    let angles: Vec<f64> = families.iter().map(|family| family.angle).collect();
    assert!(
        angles
            .iter()
            .any(|angle| (*angle - 45.0f64.to_radians()).abs() < 1e-12),
        "{angles:?}"
    );
    assert!(
        angles
            .iter()
            .any(|angle| (*angle - 135.0f64.to_radians()).abs() < 1e-12),
        "{angles:?}"
    );
    assert!(
        matches!(out.completeness, Completeness::Complete),
        "{:?}",
        out.completeness
    );
    assert_eq!(table_diagnostics(&out.diagnostics).len(), 1);
}

#[test]
fn a_padded_lowercase_name_still_resolves_and_is_kept_verbatim() {
    let hatch = rowless_hatch(
        "  ansi31\t",
        acadrust::entities::HatchPatternType::Predefined,
    );
    let out = convert(hatch);

    let pattern = imported_pattern(&out.geometry);
    assert_eq!(pattern.lines.len(), 1, "{:?}", pattern.lines);
    // Resolution trims and ignores case (it comes from the domain lookup), but
    // the imported name is still exactly what the file wrote: no rewriting.
    assert_eq!(pattern.name, "  ansi31\t");
    assert_eq!(table_diagnostics(&out.diagnostics).len(), 1);
}

#[test]
fn an_unimplemented_predefined_name_invents_no_rows_and_names_itself() {
    let hatch = rowless_hatch("ANSI33", acadrust::entities::HatchPatternType::Predefined);
    let out = convert(hatch);

    let pattern = imported_pattern(&out.geometry);
    assert!(
        pattern.lines.is_empty(),
        "no rows may be fabricated: {:?}",
        pattern.lines
    );
    let reasons = partial_reasons(&out.completeness);
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    // The existing wording is extended with the name, not replaced.
    assert!(reasons[0].contains("no line families"), "{reasons:?}");
    assert!(reasons[0].contains("ANSI33"), "{reasons:?}");
    assert!(
        reasons[0].contains("not in the implemented pattern table"),
        "{reasons:?}"
    );
    // Nothing came from the table, so no provenance claim is made.
    assert!(
        table_diagnostics(&out.diagnostics).is_empty(),
        "{:?}",
        out.diagnostics
    );
}

#[test]
fn file_rows_win_over_the_table_and_are_not_reported() {
    let mut hatch = rowless_hatch("ANSI31", acadrust::entities::HatchPatternType::Predefined);
    hatch.pattern_scale = 2.0;
    // Deliberately unlike the transcribed ANSI31 row: a horizontal family at
    // half the table's spacing, with dashes.
    hatch
        .pattern
        .add_line(acadrust::entities::HatchPatternLine {
            angle: 0.0,
            base_point: acadrust::types::Vector2::new(0.25, 0.0),
            offset: acadrust::types::Vector2::new(0.0, 0.5),
            dash_lengths: vec![0.25, -0.125],
        });
    let out = convert(hatch);

    let pattern = imported_pattern(&out.geometry);
    assert_eq!(pattern.lines.len(), 1, "{:?}", pattern.lines);
    let line = &pattern.lines[0];
    assert!((line.angle - 0.0).abs() < 1e-12, "{line:?}");
    assert!((line.base_x - 0.25).abs() < 1e-12, "{line:?}");
    assert!((line.base_y - 0.0).abs() < 1e-12, "{line:?}");
    assert!((line.offset_x - 0.0).abs() < 1e-12, "{line:?}");
    assert!((line.offset_y - 0.5).abs() < 1e-12, "{line:?}");
    assert_eq!(line.dashes, vec![0.25, -0.125]);
    assert!(
        matches!(out.completeness, Completeness::Complete),
        "{:?}",
        out.completeness
    );
    assert!(
        table_diagnostics(&out.diagnostics).is_empty(),
        "a file-defined pattern must not claim table provenance: {:?}",
        out.diagnostics
    );
}

#[test]
fn a_custom_pattern_without_rows_is_unchanged() {
    // `76=2` is defined by the file; the table must never stand in for it.
    let hatch = rowless_hatch("ANSI31", acadrust::entities::HatchPatternType::Custom);
    let out = convert(hatch);

    let pattern = imported_pattern(&out.geometry);
    assert!(
        pattern.lines.is_empty(),
        "a custom pattern gets no table rows: {:?}",
        pattern.lines
    );
    let reasons = partial_reasons(&out.completeness);
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    // Today's wording, unextended: a custom name is not looked up at all.
    assert_eq!(reasons[0], "hatch pattern has no line families");
    assert!(
        table_diagnostics(&out.diagnostics).is_empty(),
        "{:?}",
        out.diagnostics
    );
}

#[test]
fn a_writer_produced_dwg_without_rows_resolves_and_reports_provenance() {
    // End-to-end over real bytes: the file genuinely carries a `76=1` ANSI31
    // hatch with no `78` rows, so the resolution has to happen on import.
    let mut doc = acadrust::CadDocument::new();
    let mut hatch = rowless_hatch("ANSI31", acadrust::entities::HatchPatternType::Predefined);
    hatch.pattern_scale = 4.0;
    doc.add_entity(EntityType::Hatch(Box::new(hatch)))
        .expect("add hatch");
    let bytes = acadrust::DwgWriter::write_to_vec(&doc).expect("write synthetic DWG");

    let drawing = AcadrustImporter::new()
        .import(&request(bytes), &|| false)
        .expect("synthetic drawing imports");
    let reported: Vec<&Diagnostic> = drawing
        .report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == TABLE_CODE)
        .collect();
    assert_eq!(reported.len(), 1, "{:?}", drawing.report.diagnostics);
    assert!(reported[0].message.contains("ANSI31"));
    // The rows are ours, so the drawing must not look like it imported clean.
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "{:?}",
        drawing.report.completeness
    );
}
