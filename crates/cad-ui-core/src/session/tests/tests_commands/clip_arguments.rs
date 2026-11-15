//! The typed `CLIP` grammar and its dispatch.
//!
//! These pin the textual grammar (`RECT`/`POLY`/`ON`/`OFF`/`DELETE`), the
//! admission of `CLIP` as an argument form of the real extended command (it is no
//! longer a named refusal), and the payload path: a valid line with no selection
//! reaches the command layer and is refused there by name (`clip.no_selection`),
//! which proves the mapping without duplicating `cad-app`'s semantics. A
//! malformed line is refused before dispatch.
//!
//! `CLIPIT` is deliberately **not** an argument selector: its whole input is the
//! selection (one target plus the boundary entities), so a bare `CLIPIT` line is
//! the command.

use super::super::super::*;
use super::super::test_support::*;

use crate::session::commands::{parse_clip_argument, ARGUMENT_SELECTORS, CLIP_SELECTOR};
use cad_app::{ClipCommand, ExtendedCommandId, RefusalId};
use cad_domain::Point3;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn clip_argument_is_a_boundary_or_a_state_word() {
    assert_eq!(
        parse_clip_argument("RECT 0 0 10 5"),
        Some(ClipCommand::Rect([0.0, 0.0, 10.0, 5.0]))
    );
    assert_eq!(
        parse_clip_argument("rect -1.5 -2 3 4"),
        Some(ClipCommand::Rect([-1.5, -2.0, 3.0, 4.0])),
        "the keywords are case-insensitive and the corners are signed decimals"
    );
    assert_eq!(parse_clip_argument("ON"), Some(ClipCommand::On));
    assert_eq!(parse_clip_argument("off"), Some(ClipCommand::Off));
    assert_eq!(parse_clip_argument("DELETE"), Some(ClipCommand::Delete));
    assert_eq!(
        parse_clip_argument("POLY 0 0 10 0 10 5"),
        Some(ClipCommand::Polygon(vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 5.0,
                z: 0.0,
            },
        ]))
    );
    // Ordering and polygon validity are the command's named refusals, not
    // syntax: any four finite numbers or three point pairs are admitted here.
    assert_eq!(
        parse_clip_argument("RECT 10 0 0 10"),
        Some(ClipCommand::Rect([10.0, 0.0, 0.0, 10.0]))
    );
    for refused in [
        "",
        "RECT",
        "RECT 0 0 1",
        "RECT 0 0 1 1 1",
        "RECT a 0 1 1",
        "POLY 0 0 10 0",
        "POLY 0 0 10 0 10",
        "POLY 0 0 10 0 x 5",
        "ON EXTRA",
        "OFF 0",
        "DELETE 0",
        "0 0 10 5",
        "NONE",
        "REC 0 0 1 1",
    ] {
        assert_eq!(parse_clip_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn clip_is_an_argument_selector_of_the_real_extended_command() {
    assert!(ARGUMENT_SELECTORS.contains(&CLIP_SELECTOR));
    assert!(
        !ARGUMENT_SELECTORS.contains(&"CLIPIT"),
        "CLIPIT takes its whole input from the selection"
    );
    assert_eq!(
        normalize_command("CLIP"),
        Some(CommandId::Extended(ExtendedCommandId::Clip)),
        "CLIP is a real command, not the named refusal it used to be"
    );
    assert_eq!(
        normalize_command("CLIPIT"),
        Some(CommandId::Extended(ExtendedCommandId::ClipIt))
    );
    assert_eq!(canonical_command_name("CLIP"), Some("CLIP"));
    assert_eq!(canonical_command_name("CLIPIT"), Some("CLIPIT"));
    assert!(ExtendedCommandId::ALL.contains(&ExtendedCommandId::Clip));
    assert!(ExtendedCommandId::ALL.contains(&ExtendedCommandId::ClipIt));
    assert!(
        ExtendedCommandId::Clip.requires_work_mode(),
        "the CLIP write is Work-only"
    );
    assert!(
        ExtendedCommandId::ClipIt.requires_work_mode(),
        "the CLIPIT write is Work-only"
    );
    // The refusal registry never owned these two names, so the table is unchanged.
    assert!(!RefusalId::ALL.iter().any(|name| name.selector() == "CLIP"));
    assert!(!RefusalId::ALL
        .iter()
        .any(|name| name.selector() == "CLIPIT"));
}

#[test]
fn typed_clip_reaches_the_command_layer_and_refuses_no_selection() {
    let session = fresh_session();
    for line in [
        "CLIP RECT 0 0 10 5",
        "CLIP POLY 0 0 10 0 10 5",
        "CLIP ON",
        "CLIP OFF",
        "CLIP DELETE",
    ] {
        let error = session.command(line).expect_err(line);
        assert!(
            error.contains("clip.no_selection"),
            "the payload reached the command layer: {line}: {error}"
        );
    }
    // A bare selector is admitted by name and refused by the command: the
    // grammar never defaults a boundary.
    let error = session.command("CLIP").expect_err("bare CLIP");
    assert!(error.contains("clip.payload"), "{error}");
    // A bare `CLIPIT` carries no payload at all: with no selection the command
    // layer answers its own selection refusal, not a payload one.
    let error = session.command("CLIPIT").expect_err("bare CLIPIT");
    assert!(error.contains("clipit.not_clippable"), "{error}");
    assert!(!error.contains("needs no payload"), "{error}");
}

#[test]
fn malformed_clip_arguments_are_refused_before_dispatch() {
    let session = fresh_session();
    for line in [
        "CLIP RECT 0 0 1",
        "CLIP RECT 0 0 1 1 1",
        "CLIP POLY 0 0 10 0",
        "CLIP NONE",
        "CLIP 0 0 10 5",
    ] {
        let error = session.command(line).expect_err(line);
        assert!(error.starts_with("clip.argument"), "{line}: {error}");
        assert!(error.contains("nothing was changed"), "{line}: {error}");
    }
}
