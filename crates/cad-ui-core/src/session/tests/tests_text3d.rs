//! `TEXT3D` typed grammar and argument admission.
//!
//! The command writes an extruded glyph mesh, which needs a host-installed font
//! engine; the shaping/geometry contracts live in `cad-app`. Here the session only
//! has to parse the five numbers plus the string remainder and refuse a line
//! outside the grammar. Contract level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;
use crate::session::commands::parse_text3d_argument;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_text3d_grammar_takes_five_numbers_and_a_string_remainder() {
    let parsed = parse_text3d_argument("10 20 5 12 2.5 osifont.ttf Hello  World").expect("parse");
    assert_eq!(
        (parsed.origin.x, parsed.origin.y, parsed.origin.z),
        (10.0, 20.0, 5.0)
    );
    assert_eq!(parsed.height, 12.0);
    assert_eq!(parsed.extrusion, 2.5);
    assert_eq!(parsed.font.as_deref(), Some("osifont.ttf"));
    assert_eq!(parsed.text, "Hello  World", "internal spacing is preserved");
    assert_eq!(
        typed_command_argument("TEXT3D 0 0 0 10 2 - A"),
        Some(("TEXT3D", "0 0 0 10 2 - A"))
    );
    assert_eq!(command_name(CommandId::Text3d), Some("TEXT3D"));
    for refused in [
        "",
        "0 0 0 10",
        "0 0 0 10 2",
        "0 0 0 10 2",
        "0 0 0 10 2 -",
        "0 0 0 10 2 -   ",
        "a 0 0 10 2 - A",
        "0 0 0 NaN 2 - A",
        "0 0 0 10 inf - A",
    ] {
        assert_eq!(parse_text3d_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn a_text3d_line_outside_the_grammar_is_refused_before_dispatch() {
    let session = fresh_session();
    let error = session
        .command("TEXT3D 0 0 0 10 2")
        .expect_err("not enough fields");
    assert!(
        error.contains("unsupported CAD command"),
        "the grammar refusal is named: {error}"
    );
}

#[test]
fn a_well_formed_text3d_line_reaches_the_command_layer() {
    // No font engine is installed in this synthetic session, so the command's own
    // named refusal is the expected outcome — the important part is that the line
    // was parsed and dispatched rather than rejected by the grammar.
    let session = fresh_session();
    let error = session
        .command("TEXT3D 0 0 0 10 2 - A")
        .expect_err("no outline font engine in a synthetic session");
    assert!(
        error.contains("text3d.font_unavailable"),
        "the command's own refusal is named: {error}"
    );
}
