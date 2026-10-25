//! Predefined HATCH pattern contract tests (pattern type code `2`).
//!
//! Pins the three hand-transcribed patterns (`ANSI31`, `ANSI32`, `ANSI37`) end
//! to end: the definition rows stored on the created entity, the analytic line
//! count and total length of a rendered 10x10 boundary, the named refusal of an
//! index outside the table, and the export -> reimport round trip of the name,
//! rows and DXF pattern type `76=1`.
//!
//! The numbers below are **our transcribed values** (ADR U-1): no vendor file,
//! reference renderer or golden image was compared. They are exact consequences
//! of this build's `HatchPattern::families` + even-odd clip, which is what the
//! tests pin.

use super::hatch::{
    assert_refused, circle, filled_area, hatch_app, pattern_lines, run, set_flag, set_number,
    the_hatch,
};
use super::*;

use cad_domain::hatch::predefined_patterns;
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// A closed axis-aligned rectangle `[x0, x1] x [y0, y1]` (unlike [`square`] the
/// two extents differ, which keeps the 45/135 degree families off every corner
/// so a clipped chord is never zero-length).
///
/// [`square`]: super::hatch::square
fn rect(id: u128, x0: f64, x1: f64, y0: f64, y1: f64, z: f64) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Polyline {
            points: vec![p3(x0, y0, z), p3(x1, y0, z), p3(x1, y1, z), p3(x0, y1, z)],
            bulges: vec![0.0; 4],
            closed: true,
        },
    )
}

/// Configure a predefined pattern: type code `2`, the 1-based index, spacing
/// (the pattern scale / `HPSCALE`), hatch angle in degrees and the double flag.
fn predefined(session: &mut SessionState, index: f64, spacing: f64, angle: f64, double: bool) {
    set_number(
        session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_PREDEFINED,
    );
    set_number(session, ToolParameter::HatchPattern, index);
    set_number(session, ToolParameter::HatchSpacing, spacing);
    set_number(session, ToolParameter::HatchAngle, angle);
    set_flag(session, ToolParameter::HatchDouble, double);
}

#[test]
fn the_predefined_table_is_the_advertised_three_patterns() {
    assert_eq!(
        predefined_patterns()
            .iter()
            .map(|pattern| pattern.name)
            .collect::<Vec<_>>(),
        vec!["ANSI31", "ANSI32", "ANSI37"]
    );
}

#[test]
fn ansi31_stores_the_transcribed_rows_and_renders_113_diagonal_lines() {
    // The 10x10 boundary is [0.3, 10.3] x [0.7, 10.7]: the y - x offset keeps
    // every vertex off the 45 degree lines, so all 113 clipped chords are real.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    predefined(&mut session, 1.0, 1.0, 0.0, false);
    let out = run(&mut app, &mut session, &[1]).expect("HATCH succeeds");

    let hatch = the_hatch(&app);
    assert_eq!(hatch.pattern_type(), HatchPatternType::Predefined, "76=1");
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill, got {:?}", hatch.fill());
    };
    assert_eq!(pattern.name, "ANSI31");
    assert_eq!(pattern.lines.len(), 1, "ANSI31 is one definition row");
    // The row keeps the transcribed 45 degree direction and the 0.125 pattern
    // spacing; the offset is stored in the hatch-plane frame, so it is that
    // spacing expressed perpendicular to 45 degrees.
    assert!((pattern.lines[0].angle - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
    let perpendicular = 0.125 / std::f64::consts::SQRT_2;
    assert!((pattern.lines[0].offset_x + perpendicular).abs() < 1e-12);
    assert!((pattern.lines[0].offset_y - perpendicular).abs() < 1e-12);
    assert!(pattern.lines[0].dashes.is_empty(), "ANSI31 is solid");
    assert!((pattern.scale - 1.0).abs() < 1e-12);
    assert!((pattern.angle - 0.0).abs() < 1e-12);
    assert!(!pattern.double);

    // Boundary-only pattern fill: no solid mesh.
    assert_eq!(filled_area(hatch), 0.0);

    let (count, total) = pattern_lines(hatch);
    // Analytic: the 45 degree family crosses the rectangle every 0.125 units of
    // perpendicular distance. Perpendicular extent y - x is
    // [0.7 - 10.3, 10.7 - 0.3] / sqrt(2) = [-6.78823, 7.35391], so the lines
    // s = 0.125 k with k in [-54, 58] hit the boundary: 113 chords. Summing the
    // closed-form chord lengths 10*sqrt(2) - 2|s - 0.4/sqrt(2)| gives the total
    // below (the discrete family sum equals area/spacing only up to the fixed
    // comb phase, hence 799.995 640 rather than exactly 800).
    assert_eq!(count, 113, "113 diagonal chords");
    assert!(
        (total - 799.995_640_056_649_4).abs() < 1e-6,
        "total length {total}"
    );

    // The summary diagnostic names the pattern instead of claiming a
    // user-defined one (the old text would have lied).
    let message = out
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.hatch")
        .map(|diagnostic| diagnostic.message.clone())
        .expect("a hatch summary diagnostic");
    assert!(
        message.contains("predefined pattern ANSI31"),
        "diagnostic must name the pattern: {message}"
    );
    assert!(
        message.contains("associative hatch NOT IMPLEMENTED"),
        "the deferral must stay: {message}"
    );
}

#[test]
fn a_hatch_angle_of_45_turns_ansi31_into_exactly_vertical_lines() {
    // 45 (row) + 45 (hatch angle) = 90 degrees: vertical lines every 0.125 in x.
    // x in (0.3, 10.3) admits x = 0.375 .. 10.25, i.e. 80 lines of height 10.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    predefined(&mut session, 1.0, 1.0, 45.0, false);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let (count, total) = pattern_lines(the_hatch(&app));
    assert_eq!(count, 80);
    assert!((total - 800.0).abs() < 1e-9, "total length {total}");
}

#[test]
fn the_double_flag_adds_the_perpendicular_family_to_a_predefined_pattern() {
    // ANSI31 at hatch angle 0 with HPDOUBLE: the extra family is the 135 degree
    // one, so the result is the ANSI37 geometry (113 + 113 chords).
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    predefined(&mut session, 1.0, 1.0, 0.0, true);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let hatch = the_hatch(&app);
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill");
    };
    assert!(pattern.double);
    assert_eq!(pattern.lines.len(), 1, "the definition stays one row");
    let (count, total) = pattern_lines(hatch);
    assert_eq!(count, 226, "two perpendicular families");
    assert!(
        (total - 1_600.000_616_352_142_4).abs() < 1e-6,
        "total {total}"
    );
}

#[test]
fn ansi37_is_two_crossed_families_and_their_crossed_angle_composes() {
    // ANSI37 at hatch angle 0: the 45 and 135 degree families, each 113 chords.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    predefined(&mut session, 3.0, 1.0, 0.0, false);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let hatch = the_hatch(&app);
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill");
    };
    assert_eq!(pattern.name, "ANSI37");
    assert_eq!(pattern.lines.len(), 2, "two definition rows");
    let (count, total) = pattern_lines(hatch);
    assert_eq!(count, 226);
    assert!(
        (total - 1_600.000_616_352_142_4).abs() < 1e-6,
        "total {total}"
    );

    // At hatch angle 45 the rows become exactly vertical (90) and horizontal
    // (180): 80 + 80 chords of length 10.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    predefined(&mut session, 3.0, 1.0, 45.0, false);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let (count, total) = pattern_lines(the_hatch(&app));
    assert_eq!(count, 160, "80 vertical + 80 horizontal");
    assert!((total - 1600.0).abs() < 1e-9, "total length {total}");
}

#[test]
fn spacing_scales_the_family_and_ansi32_keeps_its_dash_pattern() {
    // ANSI32 (index 2) is the dashed 45 degree family; at spacing 2 the 0.375
    // pattern spacing becomes 0.75 and the dashes double.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    predefined(&mut session, 2.0, 2.0, 0.0, false);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let hatch = the_hatch(&app);
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill");
    };
    assert_eq!(pattern.name, "ANSI32");
    assert_eq!(pattern.lines.len(), 1);
    assert_eq!(pattern.lines[0].dashes, vec![0.125, -0.0625]);
    let families = pattern.families();
    assert_eq!(families.len(), 1);
    assert_eq!(
        families[0].dashes,
        vec![0.25, -0.125],
        "dashes follow scale"
    );
    let spacing = families[0].offset[0].hypot(families[0].offset[1]);
    assert!((spacing - 0.75).abs() < 1e-12, "spacing {spacing}");
    // A dashed family still draws (the clipped spans are the dash segments).
    let (count, total) = pattern_lines(hatch);
    assert!(count > 0, "the dashed family draws something");
    assert!(total > 0.0, "the dashed family has length");
}

#[test]
fn an_index_outside_the_implemented_table_is_refused_by_name() {
    // The typed channel itself already refuses index 4; the command re-validates
    // the stored value (defence in depth), so an injected out-of-range index is
    // refused by name instead of being clamped onto ANSI37.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    set_number(
        &mut session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_PREDEFINED,
    );
    session.force_tool_parameter(ToolParameter::HatchPattern, ToolValue::Number(99.0));
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        "HATCH pattern index 99 is not in the implemented predefined set",
    );
    // A fractional index is refused the same way (never truncated to an entry).
    session.force_tool_parameter(ToolParameter::HatchPattern, ToolValue::Number(2.5));
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        "HATCH pattern index 2.5 is not in the implemented predefined set",
    );
    // And the channel refuses 0 / 4 before any command runs.
    for refused in [0.0, 4.0] {
        assert!(
            session
                .set_tool_parameter(ToolParameter::HatchPattern, ToolValue::Number(refused))
                .is_err(),
            "index {refused} must be refused by the typed channel"
        );
    }
}

#[test]
fn a_created_predefined_hatch_round_trips_its_name_rows_and_76_one() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 10.0)]);
    predefined(&mut session, 3.0, 2.0, 15.0, false);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let created = the_hatch(&app).clone();
    assert_eq!(created.pattern_type(), HatchPatternType::Predefined);

    let (bytes, report) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    assert!(
        report.counts.converted >= 1,
        "a hatch is always Converted: {:?}",
        report.counts
    );
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF");

    // Walk the DXF (code, value) pairs: the HATCH record carries the pattern
    // name (group 2), the predefined pattern type (group 76 = 1) and the two
    // definition rows (group 78 = 2).
    let pairs: Vec<(&str, &str)> = text
        .lines()
        .collect::<Vec<_>>()
        .chunks(2)
        .filter_map(|chunk| match chunk {
            [code, value] => Some((code.trim(), value.trim())),
            _ => None,
        })
        .collect();
    let name_at = pairs
        .iter()
        .position(|(code, value)| *code == "2" && *value == "ANSI37")
        .expect("the pattern name is written as group 2");
    let (_, pattern_type) = pairs
        .iter()
        .skip(name_at)
        .find(|(code, _)| *code == "76")
        .expect("group 76 is written after the pattern name");
    assert_eq!(*pattern_type, "1", "a predefined pattern is 76=1");
    let (_, rows) = pairs
        .iter()
        .skip(name_at)
        .find(|(code, _)| *code == "78")
        .expect("group 78 is written");
    assert_eq!(*rows, "2", "both ANSI37 definition rows are written");

    // Re-import: the hatch stays a first-class hatch with the same definition.
    let request = ImportRequest {
        document: DocumentId(2),
        database: DatabaseId(2),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("the exported drawing re-imports");
    let (entity, hatch) = imported
        .database
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Hatch(hatch) => Some((*entity, hatch.as_ref())),
            _ => None,
        })
        .expect("the export re-imports as a first-class hatch");
    assert_eq!(entity.object.type_key, "AcDbHatch");
    assert_eq!(hatch.pattern_type(), HatchPatternType::Predefined);
    assert_eq!(hatch.loops().len(), 1);
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill, got {:?}", hatch.fill());
    };
    assert_eq!(pattern.name, "ANSI37");
    assert_eq!(pattern.lines.len(), 2, "both rows survive the round trip");
    assert!((pattern.scale - 2.0).abs() < 1e-9);
    assert!((pattern.angle - 15.0f64.to_radians()).abs() < 1e-9);
    assert!(!pattern.double);
}
