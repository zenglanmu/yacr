//! `FLIP X|Y|Z` typed grammar and dispatch.
//!
//! The axis is the only argument; the app owns the reflection. Contract level:
//! no GUI, no GPU.

use super::super::*;
use super::test_support::*;
use crate::session::commands::{parse_flip_argument, FlipArgument};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_flip_grammar_takes_one_axis_and_rejects_a_bare_line() {
    for (text, axis) in [
        ("X", FlipArgument::X),
        ("red", FlipArgument::X),
        (" Y ", FlipArgument::Y),
        ("GREEN", FlipArgument::Y),
        ("Z", FlipArgument::Z),
        ("blue", FlipArgument::Z),
    ] {
        assert_eq!(parse_flip_argument(text), Some(axis), "{text}");
    }
    for refused in ["", "  ", "Q", "X Y"] {
        assert_eq!(parse_flip_argument(refused), None, "{refused:?}");
    }

    let session = fresh_session();
    for line in ["FLIP", "FLIP Q", "FLIP X Y"] {
        let text = match session.command(line) {
            Err(text) => text,
            Ok(()) => panic!("{line} must be refused"),
        };
        assert!(text.contains("flip.axis_required"), "{line}: {text}");
    }
}

#[test]
fn flip_x_reaches_the_command_layer_with_the_red_axis() {
    let session = fresh_session();
    // The synthetic session has no document, so the app's own refusal (not the
    // grammar) proves the typed line dispatched `CommandId::Flip`.
    let text = match session.command("FLIP X") {
        Err(text) => text,
        Ok(()) => panic!("no document should refuse the flip"),
    };
    assert!(
        text.contains("document not open") || text.contains("flip.selection_required"),
        "{text}"
    );
    assert_eq!(normalize_command("FLIP"), Some(CommandId::Flip));
    assert_eq!(canonical_command_name("FLIP"), Some("FLIP"));
}
