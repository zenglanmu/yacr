//! `TOLERANCE` cell-grammar boundary contracts.
//!
//! The command is a declared **partial** implementation: it draws a faithful
//! single-row frame from plain `|`-separated cells, but it does not implement
//! the DXF GDT symbol font / `%%v` separator / `^J` multi-row grammar or the
//! dialog's full slot set. These tests pin the named refusals and the exact
//! six-cell limit, and every refusal writes nothing.

use super::*;

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
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

fn tolerance_payload(text: &str) -> CommandPayload {
    CommandPayload::Tolerance(Box::new(ToleranceCommand {
        origin: p3(0.0, 0.0, 0.0),
        text: text.into(),
    }))
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    text: &str,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            CommandId::Extended(ExtendedCommandId::Tolerance),
            tolerance_payload(text),
        ),
    )
}

#[test]
fn tolerance_refuses_the_dxf_grammar_by_name_without_writing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for (text, code) in [
        ("{\\Fgdt;p}|0.05|A", "tolerance.gdt_symbol"),
        ("A^JB", "tolerance.multiline"),
        ("A%%vB", "tolerance.dxf_separator"),
    ] {
        let error = execute(&mut app, &mut session, text)
            .expect_err("the DXF tolerance grammar is refused by name");
        assert!(error.to_string().contains(code), "{text}: {error}");
        assert_eq!(revision(&app), before);
        assert_eq!(undo_depth(&app), 0);
    }
}

#[test]
fn tolerance_accepts_six_plain_cells_and_refuses_seven() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, "A|B|C|D|E|F")
        .expect("six plain cells are the declared maximum");
    assert_eq!(
        outcome.objects.len(),
        12,
        "1 border + 5 dividers + 6 TEXT rows"
    );
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    let after_six = revision(&app);
    let error = execute(&mut app, &mut session, "A|B|C|D|E|F|G")
        .expect_err("seven cells exceed the declared single-row frame");
    assert!(error.to_string().contains("tolerance.cells"), "{error}");
    assert_eq!(revision(&app), after_six);
    assert_eq!(undo_depth(&app), 1, "the refusal adds no undo step");
}
