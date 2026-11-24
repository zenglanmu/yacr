//! The typed `PYRAMID <sides> <cx> <cy> <radius> <height>` line.
//!
//! The parser tests pin the grammar; the session tests prove the line reaches
//! the application's one create funnel (the fake host injects no modeler, so a
//! well-formed line stops at the stable `unsupported-runtime: solid-modeler`
//! refusal) and that the application's own named refusals — not the grammar —
//! answer an out-of-range value.

use super::super::super::*;
use super::super::test_support::*;
use cad_app::SolidPrimitiveCommand;

use crate::session::commands::{parse_pyramid_argument, ARGUMENT_SELECTORS, PYRAMID_SELECTOR};

fn session_with_fake_host() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn pyramid_typed_selector_is_admitted() {
    assert!(ARGUMENT_SELECTORS.contains(&PYRAMID_SELECTOR));
    assert_eq!(canonical_command_name("PYRAMID"), Some("PYRAMID"));
}

#[test]
fn pyramid_grammar_is_the_optional_ring_choice_and_five_numbers() {
    assert_eq!(
        parse_pyramid_argument("6 2 3 5 8"),
        Some(SolidPrimitiveCommand::PyramidPolygon {
            base: [2.0, 3.0],
            radius: 5.0,
            sides: 6,
            circumscribed: false,
            height: 8.0,
        }),
        "the default construction is inscribed (POLYGON's default)"
    );
    for word in ["INSCRIBED", "inscribed", "I", "i"] {
        let parsed = parse_pyramid_argument(&format!("{word} 4 0 0 2 3")).expect(word);
        assert_eq!(
            parsed,
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 2.0,
                sides: 4,
                circumscribed: false,
                height: 3.0,
            },
            "{word}"
        );
    }
    for word in ["CIRCUMSCRIBED", "circumscribed", "C", "c"] {
        let parsed = parse_pyramid_argument(&format!("{word} 5 -1 2.5 3 -4")).expect(word);
        assert_eq!(
            parsed,
            SolidPrimitiveCommand::PyramidPolygon {
                base: [-1.0, 2.5],
                radius: 3.0,
                sides: 5,
                circumscribed: true,
                height: -4.0,
            },
            "{word}: the parser does not clamp; the command layer refuses a non-positive height"
        );
    }
    for bad in [
        "",
        "6",
        "6 0 0 5",
        "6 0 0 5 8 9",
        "4.0 0 0 5 8",
        "-4 0 0 5 8",
        "6 0 0 5 8 extra",
        "SIDES 6 0 0 5 8",
        "6 nan 0 5 8",
        "6 0 0 inf 8",
    ] {
        assert_eq!(parse_pyramid_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn pyramid_typed_line_reaches_the_one_create_funnel_and_its_named_refusals() {
    let session = session_with_fake_host();
    // A well-formed line reaches the application: with no modeler injected the
    // stable carrier refusal names exactly that (proof it is not a grammar
    // refusal and not a silent success).
    let error = session.command("PYRAMID 6 0 0 5 10").unwrap_err();
    assert!(error.contains("solid-modeler"), "{error}");

    // Out-of-range values are the application's own named refusals.
    let error = session.command("PYRAMID 2 0 0 5 10").unwrap_err();
    assert!(error.contains("pyramid.sides:"), "{error}");
    let error = session.command("PYRAMID 6 0 0 0 10").unwrap_err();
    assert!(error.contains("pyramid.radius:"), "{error}");
    let error = session.command("PYRAMID 6 0 0 5 0").unwrap_err();
    assert!(error.contains("pyramid.degenerate_extent:"), "{error}");

    // A malformed line is refused before IPC by the selector's own grammar.
    let error = session.command("PYRAMID CIRCUMSCRIBED nope").unwrap_err();
    assert!(error.contains("PYRAMID takes"), "{error}");
}
