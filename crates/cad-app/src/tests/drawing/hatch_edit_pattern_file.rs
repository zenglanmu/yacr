//! `HATCHEDIT PATTERNFILE <path.pat> [<name>]` contract tests (AutoCAD 2026).
//!
//! The option applies a pattern from a **user** `.pat` file. The host reads the
//! file (the typed line is parsed in the session, which has no filesystem), so the
//! payload carries the text and these tests supply it directly — the read itself
//! is pinned in `cad-host`/`cad-ui-core`. The command parses with the one
//! `PatternLibrary` reader, keeps the hatch's stored angle/scale/`double`,
//! records the fill as a custom pattern (`76=2`) and writes one transaction / one
//! undo step. Every refusal writes nothing.

use super::hatch::{hatch_app, run, square, the_hatch, undo_depth, user_defined};
use super::*;

use crate::{HatchEditOption, PatternFileText};
use cad_domain::{HatchFill, HatchPattern, HatchPatternType};

/// A two-row custom pattern (a horizontal family at half-unit spacing and a
/// dashed vertical family), deliberately unlike every built-in one.
const LADDER: &str = "\
; a test library
*LADDER, rungs
0, 0,0, 0,.5
90, 0,0, 1,0, .25,-.25
";

fn select(session: &mut SessionState, ids: &[u128]) {
    session
        .selection
        .replace(ids.iter().copied().map(ref_of).collect::<Vec<_>>());
}

fn edit(
    app: &mut Application,
    session: &mut SessionState,
    entity: u128,
    option: HatchEditOption,
) -> CadResult<CommandOutcome> {
    select(session, &[entity]);
    execute(
        app,
        session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(option),
    )
}

fn file_option(path: &str, name: Option<&str>, text: &str) -> HatchEditOption {
    HatchEditOption::PatternFile {
        path: path.to_string(),
        name: name.map(str::to_string),
        text: Some(PatternFileText::new(text)),
    }
}

/// A pattern hatch on the closed square `id` (user-defined spacing 1, angle 0,
/// no double), so `PATTERNFILE` has definition rows to replace. A solid fill has
/// none, and no HATCHEDIT option can turn a solid into a pattern.
fn pattern_hatch(app: &mut Application, session: &mut SessionState, id: u128) -> u128 {
    user_defined(session, 1.0, 0.0, false);
    run(app, session, &[id]).expect("HATCH pattern").objects[0].0
}

fn stored_hatch(app: &Application, id: u128) -> HatchGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the hatch entity");
    match &entity.geometry {
        SemanticGeometry::Hatch(hatch) => (**hatch).clone(),
        other => panic!("expected a hatch, got {other:?}"),
    }
}

fn pattern(hatch: &HatchGeometry) -> &HatchPattern {
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill, got {:?}", hatch.fill());
    };
    pattern
}

// ---------------------------------------------------------------------------
// Applying a user `.pat` entry.
// ---------------------------------------------------------------------------

#[test]
fn a_user_pat_file_applies_its_pattern_in_one_undo() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    // A non-default angle/scale and HPDOUBLE make the "keep the stored settings"
    // contract visible: only the name and the definition rows may change.
    user_defined(&mut session, 4.0, 30.0, true);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let before = stored_hatch(&app, id);
    assert_eq!(before.pattern_type(), HatchPatternType::UserDefined);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let out = edit(
        &mut app,
        &mut session,
        id,
        file_option("/tmp/acad.pat", Some("ladder"), LADDER),
    )
    .expect("HATCHEDIT PATTERNFILE succeeds");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    assert_eq!(revision(&app).0, revision_before.0 + 1, "one revision");
    assert_eq!(
        undo_depth(&app),
        undo_before + 1,
        "one transaction / one undo step"
    );

    let after = stored_hatch(&app, id);
    assert_eq!(after.loops(), before.loops(), "loops preserved");
    assert_eq!(after.plane(), before.plane(), "plane preserved");
    assert_eq!(after.style(), before.style(), "style preserved");
    assert_eq!(after.seeds(), before.seeds(), "seeds preserved");
    assert_eq!(after.pixel_size(), before.pixel_size());
    // A user-file pattern is a **custom** pattern in the DXF sense (group 76 = 2).
    assert_eq!(after.pattern_type(), HatchPatternType::Custom);
    let stored = pattern(&after);
    assert_eq!(stored.name, "LADDER", "the file's own spelling");
    assert_eq!(stored.lines.len(), 2, "both rows came from the file");
    // The hatch's stored settings are kept verbatim (the definition supplies the
    // name and the rows only).
    assert!(
        (stored.angle - 30.0f64.to_radians()).abs() < 1e-12,
        "{}",
        stored.angle
    );
    assert_eq!(stored.scale, 4.0, "scale untouched");
    assert!(stored.double, "HPDOUBLE untouched");
    assert!(
        after
            .display()
            .iter()
            .any(|geometry| matches!(geometry, SemanticGeometry::Polyline { closed: false, .. })),
        "the display is rebuilt from the new pattern"
    );

    // One undo restores the previous fill exactly.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let restored = stored_hatch(&app, id);
    assert_eq!(restored.pattern_type(), HatchPatternType::UserDefined);
    assert_eq!(pattern(&restored), pattern(&before));
}

#[test]
fn a_single_pattern_file_needs_no_name() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let text = "*ONLY, the single pattern\n45, 0,0, 0,.125\n";
    edit(
        &mut app,
        &mut session,
        id,
        file_option("only.pat", None, text),
    )
    .expect("a one-pattern file applies without a name");
    let stored = pattern(the_hatch(&app)).clone();
    assert_eq!(stored.name, "ONLY");
    assert_eq!(stored.lines.len(), 1);
}

#[test]
fn a_name_only_in_the_builtin_table_falls_back_to_the_builtin_rows() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    // The file does not define ANSI31; the built-in table does, and the
    // classification follows the source that supplied the rows (76=1).
    let out = edit(
        &mut app,
        &mut session,
        id,
        file_option("acad.pat", Some("ANSI31"), LADDER),
    )
    .expect("the built-in table is the second source");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    assert_eq!(the_hatch(&app).pattern_type(), HatchPatternType::Predefined);
    assert_eq!(pattern(the_hatch(&app)).name, "ANSI31");
}

// ---------------------------------------------------------------------------
// Refusals (each writes nothing).
// ---------------------------------------------------------------------------

#[test]
fn a_multi_pattern_file_without_a_name_is_ambiguous() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let before = stored_hatch(&app, id);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let text = "*A, one\n0, 0,0, 0,1\n*B, two\n90, 0,0, 0,1\n";
    let error = edit(
        &mut app,
        &mut session,
        id,
        file_option("multi.pat", None, text),
    )
    .expect_err("two patterns need a name");
    let message = format!("{error:?}");
    assert!(
        message.contains("hatchedit.pattern_file_ambiguous"),
        "{message}"
    );
    assert!(message.contains("A, B"), "both names are listed: {message}");
    assert!(
        message.contains("PATTERNFILE multi.pat <name>"),
        "{message}"
    );
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored_hatch(&app, id), before);
}

#[test]
fn an_unparsable_file_is_refused_by_name() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let before = stored_hatch(&app, id);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    // A row before any header: the parser's own `hatch.pattern_file_syntax`.
    let error = edit(
        &mut app,
        &mut session,
        id,
        file_option("broken.pat", None, "0, 0,0, 0,1\n*LATE, header\n"),
    )
    .expect_err("a malformed library is refused");
    let message = format!("{error:?}");
    assert!(
        message.contains("hatchedit.pattern_file_invalid"),
        "{message}"
    );
    assert!(
        message.contains("broken.pat"),
        "the path is named: {message}"
    );
    assert!(
        message.contains("hatch.pattern_file_syntax"),
        "the parser's own refusal is preserved: {message}"
    );
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored_hatch(&app, id), before);
}

#[test]
fn a_file_with_no_pattern_definition_is_refused_by_name() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let undo_before = undo_depth(&app);
    for text in ["", "; only a comment\n", "*NOROWS, a header with no rows\n"] {
        let error = edit(
            &mut app,
            &mut session,
            id,
            file_option("empty.pat", None, text),
        )
        .expect_err("a library with no usable definition is refused");
        let message = format!("{error:?}");
        assert!(
            message.contains("hatchedit.pattern_file_invalid"),
            "{text:?}: {message}"
        );
    }
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(pattern(the_hatch(&app)).name, "USER", "nothing was applied");
}

#[test]
fn an_unknown_name_names_both_sources() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let before = stored_hatch(&app, id);
    let error = edit(
        &mut app,
        &mut session,
        id,
        file_option("acad.pat", Some("NOSUCH"), LADDER),
    )
    .expect_err("a name in neither source is refused");
    let message = format!("{error:?}");
    assert!(message.contains("hatchedit.unknown_pattern"), "{message}");
    assert!(
        message.contains("neither in acad.pat"),
        "the file is named: {message}"
    );
    assert!(message.contains("LADDER"), "the file's names: {message}");
    assert!(message.contains("ANSI31"), "the built-in names: {message}");
    assert_eq!(stored_hatch(&app, id), before);
}

#[test]
fn a_file_pattern_on_a_solid_fill_is_refused() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    // The creation default is a solid fill, which has no definition rows to
    // replace.
    let id = run(&mut app, &mut session, &[1])
        .expect("HATCH solid")
        .objects[0]
        .0;
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = edit(
        &mut app,
        &mut session,
        id,
        file_option("acad.pat", Some("LADDER"), LADDER),
    )
    .expect_err("a solid fill is not a pattern");
    let message = format!("{error:?}");
    assert!(
        message.contains("hatchedit.pattern_requires_pattern"),
        "{message}"
    );
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn a_payload_without_the_host_read_is_refused_and_never_reads_a_file() {
    // Only a payload built outside the session can carry no text. The command
    // must refuse by name: reading the path itself is the host's job.
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let undo_before = undo_depth(&app);
    let error = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::PatternFile {
            path: "/nonexistent/acad.pat".into(),
            name: None,
            text: None,
        },
    )
    .expect_err("a payload without the host read is refused");
    let message = format!("{error:?}");
    assert!(
        message.contains("hatchedit.pattern_file_not_read"),
        "{message}"
    );
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(pattern(the_hatch(&app)).name, "USER", "nothing was applied");

    // An empty path is its own refusal (the grammar refuses a bare
    // `PATTERNFILE` before this point).
    let error = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::PatternFile {
            path: "  ".into(),
            name: None,
            text: Some(PatternFileText::new(LADDER)),
        },
    )
    .expect_err("an empty path is refused");
    assert!(
        format!("{error:?}").contains("hatchedit.pattern_file_path"),
        "{error:?}"
    );
    assert_eq!(undo_depth(&app), undo_before);
}
