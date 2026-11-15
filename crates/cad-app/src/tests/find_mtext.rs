//! `FIND` over MTEXT formatting runs (2026-11-15).
//!
//! The search haystack for MTEXT is the plain text the viewport draws, produced
//! by the same tokenizer that renders it
//! (`cad_representation::mtext_projection`); a replacement is spliced back into
//! the markup so every untouched formatting run keeps its own codes.
//!
//! Pinned here: a match split across two runs, a `\P` paragraph break matched as
//! a break (never as the literal `\P`), a needle that exists only in markup not
//! reported, ASCII case-insensitivity, one undo step for a replace across
//! entities, and an `AcDbText` keeping the verbatim (raw-string) behaviour.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

const MTEXT_BOLD: EntityId = EntityId(21);
const MTEXT_BREAK: EntityId = EntityId(22);
const MTEXT_GOLD: EntityId = EntityId(23);
const TEXT_RAW: EntityId = EntityId(24);

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

/// Two formatted MTEXT values, one with a `\P` break, and a plain TEXT:
///
/// * `MTEXT_BOLD` draws `Bold plain` (`Bold` is inside `\fArial|b1;`);
/// * `MTEXT_BREAK` draws `first` / `second`;
/// * `MTEXT_GOLD` draws `gold tail` (`gold` is inside `\fArial|b1;`);
/// * `TEXT_RAW` is one stored line, `Cold text`.
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
    ] {
        b.insert_entity(entity(id, "AcDbMText", text(value, x)))
            .unwrap();
    }
    b.insert_entity(entity(TEXT_RAW, "AcDbText", text("Cold text", 30.0)))
        .unwrap();
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
    // The other MTEXT values are untouched: no cross-entity leakage.
    assert_eq!(stored(&app, MTEXT_BREAK), "first\\Psecond");
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

/// The existing option — ASCII case-insensitive search — applies to the plain
/// text of an MTEXT too.
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

/// A replace that hits two MTEXT values and one TEXT is one transaction and one
/// undo step; each MTEXT keeps its own run codes, the TEXT keeps raw bytes.
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

/// A non-MTEXT TEXT keeps today's behaviour: a verbatim substring replacement
/// with no escaping and no markup parsing, and a line break is still refused
/// before any transaction.
#[test]
fn find_keeps_verbatim_behaviour_for_an_acdbtext_entity() {
    let (mut app, mut session) = find_app();
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "text",
        Some("x{y}"),
    )
    .unwrap();
    // Stored exactly as given: a TEXT is one line, not MTEXT markup.
    assert_eq!(stored(&app, TEXT_RAW), "Cold x{y}");
    assert_eq!(stored(&app, MTEXT_BOLD), "{\\fArial|b1;Bold} plain");

    let revision_before = revision(&app);
    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "Cold",
        Some("a\nb"),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.contains("text.single_line_newline")),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(stored(&app, TEXT_RAW), "Cold x{y}");
}
