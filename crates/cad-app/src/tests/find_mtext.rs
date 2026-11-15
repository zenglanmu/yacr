//! `FIND` over the text a stored value **draws** (2026-11-15).
//!
//! The search haystack for every `Text` geometry — MTEXT, MULTILEADER text and
//! single-line `AcDbText` — is the plain-text projection produced by the same
//! tokenizer that renders it (`cad_representation::mtext_projection`); a
//! replacement is spliced back into the markup so every untouched formatting run
//! keeps its own codes, and the cases the control-content grammar cannot express
//! are refused by name instead of written approximately.
//!
//! Pinned here: a match split across two runs, a `\P` paragraph break matched as
//! a break (never as the literal `\P`), a needle that exists only in markup not
//! reported, ASCII case-insensitivity, one undo step for a replace across
//! entities, an `AcDbText` matched on its drawn `%%d`/`\~` escapes, the
//! `find.mtext_partial_code` refusal for a partial `\S` fraction and a partial
//! `^I` tab, and the `find.mtext_replacement_not_literal` refusal.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

const MTEXT_BOLD: EntityId = EntityId(21);
const MTEXT_BREAK: EntityId = EntityId(22);
const MTEXT_GOLD: EntityId = EntityId(23);
const TEXT_RAW: EntityId = EntityId(24);
const TEXT_ESCAPE: EntityId = EntityId(25);
const MTEXT_FRACTION: EntityId = EntityId(26);
const MTEXT_TAB: EntityId = EntityId(27);

fn entity(id: EntityId, type_key: &str, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn text(value: &str, x: f64) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: value.into(),
        position: point(x, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
    }
}

/// Formatted and code-bearing text values, each with the drawn text in a comment:
///
/// * `MTEXT_BOLD`                stores `{\fArial|b1;Bold} plain` → `Bold plain`
/// * `MTEXT_BREAK`               stores `first\Psecond`         → `first` / `second`
/// * `MTEXT_GOLD`                stores `{\fArial|b1;gold} tail` → `gold tail`
/// * `TEXT_RAW` (`AcDbText`)     stores `Cold text`             → `Cold text`
/// * `TEXT_ESCAPE` (`AcDbText`)  stores `20%%d hot\~ok`         → `20° hot<nbsp>ok`
/// * `MTEXT_FRACTION`            stores `x\S1/2;y`              → `x1/2y`
/// * `MTEXT_TAB`                 stores `a^Ib`                  → `a    b`
fn find_app() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (id, value, x) in [
        (MTEXT_BOLD, "{\\fArial|b1;Bold} plain", 0.0),
        (MTEXT_BREAK, "first\\Psecond", 10.0),
        (MTEXT_GOLD, "{\\fArial|b1;gold} tail", 20.0),
        (MTEXT_FRACTION, "x\\S1/2;y", 40.0),
        (MTEXT_TAB, "a^Ib", 50.0),
    ] {
        b.insert_entity(entity(id, "AcDbMText", text(value, x)))
            .unwrap();
    }
    for (id, value, x) in [
        (TEXT_RAW, "Cold text", 30.0),
        (TEXT_ESCAPE, "20%%d hot\\~ok", 35.0),
    ] {
        b.insert_entity(entity(id, "AcDbText", text(value, x)))
            .unwrap();
    }
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    needle: &str,
    replacement: Option<&str>,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            id,
            CommandPayload::TextFind {
                needle: needle.into(),
                replacement: replacement.map(str::to_string),
            },
        ),
    )
}

/// The `find.result` report message.
fn report_message(outcome: &CommandOutcome) -> &str {
    &outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "find.result")
        .expect("a find report")
        .message
}

fn stored(app: &Application, id: EntityId) -> String {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .expect("entity exists")
        .geometry
    {
        SemanticGeometry::Text { text, .. } => text.clone(),
        other => panic!("expected a text geometry, got {other:?}"),
    }
}

fn shown(app: &Application, id: EntityId) -> String {
    cad_representation::mtext_projection(&stored(app, id)).text()
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

/// A needle split across a formatting-run boundary matches, and the replace
/// keeps the surrounding run codes: `\fArial|b1;` still opens the run, and the
/// `}` that closed it stays where the match started so the inserted text takes
/// that run's formatting.
#[test]
fn find_matches_plain_text_across_mtext_runs_and_keeps_the_run_codes() {
    let (mut app, mut session) = find_app();
    // `Bold` (run 1) + ` plain` (run 2): the needle starts in run 1 and
    // continues into run 2.
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "d pl", None).unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("1 match(es)"), "{message}");
    // The report shows the plain-text context, not `{\fArial|b1;Bold} plain`.
    assert!(message.contains("first: d plain"), "{message}");
    assert!(!message.contains("Arial"), "{message}");

    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "d pl",
        Some("X"),
    )
    .unwrap();
    assert_eq!(stored(&app, MTEXT_BOLD), "{\\fArial|b1;BolX}ain");
    assert_eq!(shown(&app, MTEXT_BOLD), "BolXain");
    // The other text values are untouched: no cross-entity leakage.
    assert_eq!(stored(&app, MTEXT_BREAK), "first\\Psecond");
    assert_eq!(stored(&app, MTEXT_GOLD), "{\\fArial|b1;gold} tail");
}

/// `\P` is a paragraph break in the drawn text: a needle containing a line
/// break matches it, the literal `\P` does not.
#[test]
fn find_matches_mtext_paragraph_breaks_as_breaks_not_as_markup() {
    let (mut app, mut session) = find_app();

    let outcome = run(
        &mut app,
        &mut session,
        CommandId::FindReport,
        "first\nsecond",
        None,
    )
    .unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("1 match(es)"), "{message}");

    // The stored `\P` is markup, so the two-character needle is not a match.
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "\\P", None).unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("0 match(es)"), "{message}");

    // Replacing one paragraph keeps the stored break.
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "first",
        Some("1st"),
    )
    .unwrap();
    assert_eq!(stored(&app, MTEXT_BREAK), "1st\\Psecond");
    assert_eq!(shown(&app, MTEXT_BREAK), "1st\nsecond");

    // Replacing the whole two-paragraph needle consumes the break with it.
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "1st\nsecond",
        Some("only"),
    )
    .unwrap();
    assert_eq!(stored(&app, MTEXT_BREAK), "only");
}

/// A needle that only exists in the markup (inside the font code) used to be a
/// false positive over the raw string; it is not reported now.
#[test]
fn find_does_not_report_a_needle_that_only_exists_in_mtext_markup() {
    let (mut app, mut session) = find_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    // `|b1;` is part of `\fArial|b1;` on two MTEXT values.
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "|b1;", None).unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("0 match(es)"), "{message}");

    // A replace has nothing to do: no write, no undo entry.
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "|b1;",
        Some("X"),
    )
    .unwrap();
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored(&app, MTEXT_BOLD), "{\\fArial|b1;Bold} plain");
    assert_eq!(stored(&app, MTEXT_GOLD), "{\\fArial|b1;gold} tail");
}

/// The existing option — ASCII case-insensitive search — applies to the drawn
/// text of an MTEXT.
#[test]
fn find_is_case_insensitive_on_mtext_plain_text() {
    let (mut app, mut session) = find_app();
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "BOLD", None).unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("1 match(es)"), "{message}");
    assert!(message.contains("first: Bold plain"), "{message}");

    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "bOlD",
        Some("Cool"),
    )
    .unwrap();
    assert_eq!(stored(&app, MTEXT_BOLD), "{\\fArial|b1;Cool} plain");
    assert_eq!(shown(&app, MTEXT_BOLD), "Cool plain");
}

/// A replace that hits two MTEXT values and one `AcDbText` is one transaction and
/// one undo step; each formatted MTEXT keeps its own run codes.
#[test]
fn find_replace_across_entities_is_one_undo_step() {
    let (mut app, mut session) = find_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let outcome = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "ol",
        Some("XX"),
    )
    .unwrap();
    assert!(outcome.changes.is_some());
    assert_eq!(undo_depth(&app), undo_before + 1);
    assert_ne!(revision(&app), revision_before);
    // Both formatted runs survive with only the matched characters changed.
    assert_eq!(stored(&app, MTEXT_BOLD), "{\\fArial|b1;BXXd} plain");
    assert_eq!(stored(&app, MTEXT_GOLD), "{\\fArial|b1;gXXd} tail");
    assert_eq!(stored(&app, TEXT_RAW), "CXXd text");
    // `first\Psecond` has no `ol`.
    assert_eq!(stored(&app, MTEXT_BREAK), "first\\Psecond");

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(stored(&app, MTEXT_BOLD), "{\\fArial|b1;Bold} plain");
    assert_eq!(stored(&app, MTEXT_GOLD), "{\\fArial|b1;gold} tail");
    assert_eq!(stored(&app, TEXT_RAW), "Cold text");
    assert_eq!(undo_depth(&app), undo_before);
}

/// An `AcDbText` is drawn through the same parser as MTEXT, so it is matched on
/// its drawn characters: `%%d`/`\~` are searched as `°`/a non-breaking space, and
/// a replacement is escaped so it draws exactly what was typed. A stored line
/// break in the replacement is still refused by the single-line policy.
#[test]
fn find_matches_an_acdbtext_on_its_drawn_escapes() {
    let (mut app, mut session) = find_app();

    // `20%%d hot\~ok` draws `20° hot<nbsp>ok`.
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "°", None).unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("1 match(es)"), "{message}");
    assert!(message.contains("first: ° hot"), "{message}");

    // The escape itself is markup, not a match.
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "%%d", None).unwrap();
    assert!(
        report_message(&outcome).contains("0 match(es)"),
        "{}",
        report_message(&outcome)
    );

    // Replacing the drawn `°` rewrites the escape that produced it.
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "°",
        Some("deg"),
    )
    .unwrap();
    assert_eq!(stored(&app, TEXT_ESCAPE), "20deg hot\\~ok");
    assert_eq!(shown(&app, TEXT_ESCAPE), "20deg hot\u{00A0}ok");

    // The drawn non-breaking space finds the `\~` escape.
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "\u{00A0}",
        Some(" "),
    )
    .unwrap();
    assert_eq!(stored(&app, TEXT_ESCAPE), "20deg hot ok");

    // A grouping brace in the replacement is escaped, so it draws literally.
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "Cold",
        Some("x{y}"),
    )
    .unwrap();
    assert_eq!(stored(&app, TEXT_RAW), "x\\{y\\} text");
    assert_eq!(shown(&app, TEXT_RAW), "x{y} text");

    // A single-line TEXT still refuses a stored line break, before any write.
    let revision_before = revision(&app);
    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "text",
        Some("a\nb"),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.contains("text.single_line_newline")),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(stored(&app, TEXT_RAW), "x\\{y\\} text");
}

/// Replacing only part of a `\S` stacked fraction is refused by name: the code
/// has no partial form, so the fraction would silently become plain text.
#[test]
fn find_refuses_a_partial_match_inside_a_stacked_fraction() {
    let (mut app, mut session) = find_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    // `x\S1/2;y` draws `x1/2y`; the fraction's own text matches…
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "1/2", None).unwrap();
    assert!(
        report_message(&outcome).contains("1 match(es)"),
        "{}",
        report_message(&outcome)
    );

    // …but replacing only its numerator would drop the `\S` structure.
    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "1",
        Some("10"),
    )
    .unwrap_err();
    match &error {
        CadError::Unsupported(message) => {
            assert!(
                message.starts_with("find.mtext_partial_code: "),
                "{message}"
            );
            assert!(message.contains("\\S1/2;"), "{message}");
            assert!(message.contains("stacked fraction"), "{message}");
            assert!(message.contains("bytes 1..7"), "{message}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    // Nothing was written and no undo entry was recorded.
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored(&app, MTEXT_FRACTION), "x\\S1/2;y");

    // Replacing all of its drawn text is allowed: the user replaced everything
    // the code stood for, so it is stored as literal text.
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "1/2",
        Some("3/4"),
    )
    .unwrap();
    assert_eq!(stored(&app, MTEXT_FRACTION), "x3/4y");
    assert_eq!(shown(&app, MTEXT_FRACTION), "x3/4y");
}

/// `^I` draws four spaces and has no "tab plus content" form, so replacing only
/// some of them is refused by name; replacing all four is allowed.
#[test]
fn find_refuses_a_partial_match_inside_a_tab_code() {
    let (mut app, mut session) = find_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    // `a^Ib` draws `a    b`; three of those four spaces are part of one token.
    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "   ",
        Some("--"),
    )
    .unwrap_err();
    match &error {
        CadError::Unsupported(message) => {
            assert!(
                message.starts_with("find.mtext_partial_code: "),
                "{message}"
            );
            assert!(message.contains("^I"), "{message}");
            assert!(message.contains("stored tab"), "{message}");
            assert!(message.contains("bytes 1..3"), "{message}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored(&app, MTEXT_TAB), "a^Ib");

    // The whole tab's drawn text may be replaced.
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "    ",
        Some("|"),
    )
    .unwrap();
    assert_eq!(stored(&app, MTEXT_TAB), "a|b");
    assert_eq!(shown(&app, MTEXT_TAB), "a|b");
}

/// A replacement the control-content grammar would draw differently is refused
/// by name rather than stored as text that draws something else.
#[test]
fn find_refuses_a_replacement_that_the_markup_would_redraw() {
    let (mut app, mut session) = find_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    // `%%d` inside a replacement would be drawn as `°`, not as typed.
    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "plain",
        Some("a%%db"),
    )
    .unwrap_err();
    match &error {
        CadError::InvalidInput(message) => {
            assert!(
                message.starts_with("find.mtext_replacement_not_literal: "),
                "{message}"
            );
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored(&app, MTEXT_BOLD), "{\\fArial|b1;Bold} plain");

    // The same rule protects an `AcDbText`.
    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "Cold",
        Some("a^Ib"),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.contains("find.mtext_replacement_not_literal")),
        "{error:?}"
    );
    assert_eq!(stored(&app, TEXT_RAW), "Cold text");
}
