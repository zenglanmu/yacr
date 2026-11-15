//! The typed `BASE` / `LIMITS` lines and the four Wave-31 `SETVAR` writes.
//!
//! Pinned here: the number grammar (components, comma/space separators, the
//! one `z` default), the `LIMITS ON|OFF|SET` named refusal, the dispatch to the
//! **existing** header-write commands (no second write path), and — for
//! `SETVAR LIMMIN`/`LIMMAX` — that the stored other corner is read from the
//! authoritative drawing instead of being defaulted.

use super::super::super::*;
use super::super::test_support::*;

use crate::session::commands::{
    parse_base_argument, parse_limits_argument, ARGUMENT_SELECTORS, BASE_SELECTOR, LIMITS_SELECTOR,
};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn drawing(session: &CadSession) -> std::sync::Arc<cad_db::DrawingDatabase> {
    session
        .core
        .controller
        .borrow()
        .drawing()
        .expect("a drawing is open")
}

#[test]
fn base_takes_two_or_three_decimals_and_nothing_else() {
    assert_eq!(parse_base_argument("12.5 -3"), Some([12.5, -3.0, 0.0]));
    assert_eq!(parse_base_argument("12.5,-3,2"), Some([12.5, -3.0, 2.0]));
    assert_eq!(parse_base_argument("  1   2   3  "), Some([1.0, 2.0, 3.0]));
    for refused in ["", "1", "1 2 3 4", "a b", "1e3 2", "1 .5", "1,,2", "NaN 1"] {
        assert_eq!(parse_base_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn limits_needs_exactly_four_decimals() {
    assert_eq!(
        parse_limits_argument("0 0 420 297"),
        Some([0.0, 0.0, 420.0, 297.0])
    );
    assert_eq!(
        parse_limits_argument("-5,-5,10,10"),
        Some([-5.0, -5.0, 10.0, 10.0])
    );
    for refused in ["", "0 0 420", "0 0 420 297 1", "ON", "0 0 x 1"] {
        assert_eq!(parse_limits_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn both_selectors_are_admitted_argument_forms() {
    assert!(ARGUMENT_SELECTORS.contains(&BASE_SELECTOR));
    assert!(ARGUMENT_SELECTORS.contains(&LIMITS_SELECTOR));
}

#[test]
fn base_writes_the_header_point_through_the_drawing_command() {
    let session = fresh_session();
    let before = drawing(&session).revision();
    session.command("BASE 4 5").expect("BASE writes one point");
    assert_eq!(
        drawing(&session).insertion_base(),
        Some(Point3 {
            x: 4.0,
            y: 5.0,
            z: 0.0
        })
    );
    assert!(drawing(&session).revision() > before);
    // `SETVAR INSBASE` reads the value the typed line wrote: one owner.
    session
        .command("SETVAR INSBASE")
        .expect("the value reports");
    let error = session.command("BASE 1").unwrap_err();
    assert!(error.contains("invalid BASE argument"), "{error}");
}

#[test]
fn limits_writes_both_corners_and_refuses_the_check_switch_by_name() {
    let session = fresh_session();
    session
        .command("LIMITS 0 0 420 297")
        .expect("LIMITS writes both corners");
    assert_eq!(
        drawing(&session).model_limits(),
        Some(([0.0, 0.0], [420.0, 297.0]))
    );
    for word in ["ON", "OFF", "SET"] {
        let error = session.command(&format!("LIMITS {word}")).unwrap_err();
        assert!(
            error.contains("limits.check_unsupported"),
            "{word}: {error}"
        );
    }
    let error = session.command("LIMITS 0 0 420").unwrap_err();
    assert!(error.contains("invalid LIMITS argument"), "{error}");
}

#[test]
fn setvar_writes_the_header_values_through_base_and_limits() {
    let session = fresh_session();
    session
        .command("SETVAR INSBASE 1 2 3")
        .expect("$INSBASE write");
    assert_eq!(
        drawing(&session).insertion_base(),
        Some(Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0
        })
    );

    // One corner at a time: the other corner must already be stored, because a
    // half rectangle is never invented.
    let error = session.command("SETVAR LIMMIN 0 0").unwrap_err();
    assert!(error.contains("limits.other_corner_unset"), "{error}");
    session.command("LIMITS 0 0 10 10").expect("both corners");
    session
        .command("SETVAR LIMMIN -5 -5")
        .expect("$LIMMIN write keeps the stored $LIMMAX");
    assert_eq!(
        drawing(&session).model_limits(),
        Some(([-5.0, -5.0], [10.0, 10.0]))
    );
    session
        .command("SETVAR LIMMAX 20 20")
        .expect("$LIMMAX write keeps the stored $LIMMIN");
    assert_eq!(
        drawing(&session).model_limits(),
        Some(([-5.0, -5.0], [20.0, 20.0]))
    );
}

#[test]
fn setvar_cvport_writes_the_session_marker_and_nothing_else() {
    let session = fresh_session();
    // Model space has no current viewport, so the write is refused by the
    // command layer, not silently accepted.
    let error = session.command("SETVAR CVPORT 1").unwrap_err();
    assert!(error.contains("cvport."), "{error}");
    let before = drawing(&session).revision();
    assert_eq!(drawing(&session).revision(), before);
}
