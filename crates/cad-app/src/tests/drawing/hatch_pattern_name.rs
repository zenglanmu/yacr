//! `HATCH PATTERN <name>` / `HATCH PATTERNFILE <path.pat> [<name>]` creation
//! contracts.
//!
//! Pins the **behaviour** the session-side grammar tests cannot see: the stored
//! pattern name / DXF type / scale / angle / double of the created hatch, that a
//! user `.pat` definition becomes a `Custom` (`76 = 2`) pattern, the one
//! transaction / one undo step, and one refusal per named code proving nothing
//! was written.

use super::*;

use super::hatch::{
    circle, entity_count, hatch_app, pattern_lines, set_flag, set_number, the_hatch, undo_depth,
};
use crate::command_id::hatch::HatchPatternRequest;
use crate::{PatternFileText, HATCH_PATTERN_TYPE_SOLID};
use cad_domain::{HatchFill, HatchPatternType};

/// Run one typed-pattern HATCH over `ids`.
fn run_pattern(
    app: &mut Application,
    session: &mut SessionState,
    ids: &[u128],
    request: HatchPatternRequest,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Hatch,
        CommandPayload::HatchPattern {
            refs: ids.iter().copied().map(ref_of).collect(),
            request,
        },
    )
}

/// The named `.pat` definition as a file request (the session would carry the
/// host-read text; here the test is the host).
fn file(path: &str, name: Option<&str>, text: &str) -> HatchPatternRequest {
    HatchPatternRequest::File {
        path: path.into(),
        name: name.map(str::to_string),
        text: PatternFileText::new(text),
    }
}

/// Assert a refusal that names `needle` and changed nothing.
fn assert_refused(
    app: &mut Application,
    session: &mut SessionState,
    ids: &[u128],
    request: HatchPatternRequest,
    needle: &str,
) {
    let before = revision(app);
    let depth = undo_depth(app);
    let entities = entity_count(app);
    let error = run_pattern(app, session, ids, request).expect_err("the command must refuse");
    let text = format!("{error:?}");
    assert!(text.contains(needle), "expected {needle:?} in {text}");
    assert_eq!(revision(app), before, "a refusal must not write");
    assert_eq!(undo_depth(app), depth, "a refusal must open no transaction");
    assert_eq!(entity_count(app), entities, "a refusal inserts nothing");
}

// ---------------------------------------------------------------------------
// Built-in name
// ---------------------------------------------------------------------------

#[test]
fn a_named_builtin_pattern_stores_the_library_name_type_scale_angle_and_double() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    // The session channel still supplies the pattern scale/angle/double; the
    // typed line only decides **which** pattern.
    set_number(&mut session, ToolParameter::HatchSpacing, 2.0);
    set_number(&mut session, ToolParameter::HatchAngle, 45.0);
    set_flag(&mut session, ToolParameter::HatchDouble, true);
    // The session fill type is left at its solid default on purpose: a typed
    // pattern name must not be refused (or overridden) by it.
    assert_eq!(
        session.tool_parameter(ToolParameter::HatchPatternType),
        ToolValue::Number(HATCH_PATTERN_TYPE_SOLID)
    );

    let before = revision(&app);
    let outcome = run_pattern(
        &mut app,
        &mut session,
        &[1],
        HatchPatternRequest::Named("ansi31".into()),
    )
    .expect("HATCH PATTERN ANSI31 succeeds");
    assert_eq!(outcome.objects.len(), 1, "one hatch entity");
    assert_eq!(revision(&app).0, before.0 + 1, "one revision bump");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    let hatch = the_hatch(&app);
    assert_eq!(
        hatch.pattern_type(),
        HatchPatternType::Predefined,
        "a built-in library entry is DXF group 76 = 1"
    );
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("the fill must be a pattern");
    };
    assert_eq!(
        pattern.name, "ANSI31",
        "the library name is stored verbatim"
    );
    assert_eq!(pattern.scale, 2.0, "the scale is the session hatchSpacing");
    assert!(
        (pattern.angle - 45f64.to_radians()).abs() < 1e-12,
        "the angle is the session hatchAngle, radians in the domain: {}",
        pattern.angle
    );
    assert!(
        pattern.double,
        "the perpendicular family follows hatchDouble"
    );
    assert!(
        !pattern.lines.is_empty(),
        "the parsed definition rows are carried"
    );
    // The display is derived from the stored rows (a pattern fill, no solid mesh).
    let (lines, length) = pattern_lines(hatch);
    assert!(lines > 0, "the derived display draws pattern lines");
    assert!(length > 0.0, "the pattern lines have length: {length}");
}

#[test]
fn the_typed_creation_is_one_undo_step() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    run_pattern(
        &mut app,
        &mut session,
        &[1],
        HatchPatternRequest::Named("ANSI37".into()),
    )
    .expect("HATCH PATTERN ANSI37 succeeds");
    assert_eq!(undo_depth(&app), 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 1, "only the boundary entity remains");
    assert_eq!(undo_depth(&app), 0);
}

// ---------------------------------------------------------------------------
// User `.pat` file
// ---------------------------------------------------------------------------

#[test]
fn a_user_file_definition_is_stored_as_a_custom_pattern() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    set_number(&mut session, ToolParameter::HatchSpacing, 0.5);
    set_number(&mut session, ToolParameter::HatchAngle, 0.0);
    let outcome = run_pattern(
        &mut app,
        &mut session,
        &[1],
        file("/tmp/acad.pat", None, "*MINE, one\n0, 0,0, 0,1\n"),
    )
    .expect("a one-pattern file needs no name");
    assert_eq!(outcome.objects.len(), 1);
    let hatch = the_hatch(&app);
    assert_eq!(
        hatch.pattern_type(),
        HatchPatternType::Custom,
        "a user-file definition is DXF group 76 = 2"
    );
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("the fill must be a pattern");
    };
    assert_eq!(pattern.name, "MINE");
    assert_eq!(pattern.scale, 0.5);
    assert_eq!(pattern.lines.len(), 1, "the file's one row is carried");
    // 0.5 scale over the 1.0 row spacing: the display has the derived family.
    let (lines, length) = pattern_lines(hatch);
    assert!(lines > 0 && length > 0.0, "{lines} lines, {length} long");
}

#[test]
fn a_named_entry_of_a_multi_pattern_file_is_selected() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    let text = "*FIRST, one\n0, 0,0, 0,1\n*SECOND, two rows\n45, 0,0, 0,2\n135, 0,0, 0,2\n";
    run_pattern(
        &mut app,
        &mut session,
        &[1],
        file("/tmp/two.pat", Some("second"), text),
    )
    .expect("the named entry is selected");
    let HatchFill::Pattern(pattern) = the_hatch(&app).fill() else {
        panic!("the fill must be a pattern");
    };
    assert_eq!(pattern.name, "SECOND");
    assert_eq!(pattern.lines.len(), 2);
}

#[test]
fn a_builtin_name_inside_a_user_file_request_falls_back_to_the_builtin_table() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    // The file is searched first, the built-in table second (the `HATCHEDIT
    // PATTERNFILE` rule); a file that does not hold the name must not refuse it.
    run_pattern(
        &mut app,
        &mut session,
        &[1],
        file("/tmp/acad.pat", Some("ANSI32"), "*MINE, one\n0, 0,0, 0,1\n"),
    )
    .expect("the built-in fallback resolves ANSI32");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.pattern_type(), HatchPatternType::Predefined);
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("the fill must be a pattern");
    };
    assert_eq!(pattern.name, "ANSI32");
}

// ---------------------------------------------------------------------------
// Named refusals (one per code; nothing written)
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_builtin_name_is_refused_by_name_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        HatchPatternRequest::Named("NOPE".into()),
        "hatch.unknown_pattern",
    );
    // The refusal lists the implemented table.
    let error = run_pattern(
        &mut app,
        &mut session,
        &[1],
        HatchPatternRequest::Named("NOPE".into()),
    )
    .expect_err("unknown name");
    let text = format!("{error:?}");
    assert!(text.contains("ANSI31"), "{text}");
}

#[test]
fn a_missing_pattern_name_is_refused_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        HatchPatternRequest::Named("  ".into()),
        "hatch.needs_pattern_name",
    );
}

#[test]
fn an_unknown_name_in_a_file_request_names_both_sources() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    let request = file("/tmp/acad.pat", Some("NOPE"), "*MINE, one\n0, 0,0, 0,1\n");
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        request.clone(),
        "hatch.unknown_pattern",
    );
    let error = run_pattern(&mut app, &mut session, &[1], request).expect_err("unknown name");
    let text = format!("{error:?}");
    assert!(text.contains("/tmp/acad.pat"), "the file is named: {text}");
    assert!(
        text.contains("built-in"),
        "the second source is named: {text}"
    );
}

#[test]
fn an_unparsable_pattern_file_is_refused_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        file("/tmp/bad.pat", None, "garbage without a header\n"),
        "hatch.pattern_file_invalid",
    );
}

#[test]
fn an_ambiguous_pattern_file_is_refused_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        file(
            "/tmp/two.pat",
            None,
            "*FIRST, one\n0, 0,0, 0,1\n*SECOND, two\n0, 0,0, 0,2\n",
        ),
        "hatch.pattern_file_ambiguous",
    );
}

#[test]
fn an_empty_boundary_selection_is_refused_by_name_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    assert_refused(
        &mut app,
        &mut session,
        &[],
        HatchPatternRequest::Named("ANSI31".into()),
        "hatch.selection",
    );
}

#[test]
fn the_typed_pattern_line_is_work_only() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let error = run_pattern(
        &mut app,
        &mut session,
        &[1],
        HatchPatternRequest::Named("ANSI31".into()),
    )
    .expect_err("Viewer must refuse");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before, "a Viewer refusal writes nothing");
    assert_eq!(undo_depth(&app), 0);
}
