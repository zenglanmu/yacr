//! `SHAPE` / `MINSERT` / `ADDSELECTED` typed-argument grammars.
//!
//! The parsers are pure and strict; the command layer is the authority for every
//! semantic refusal (`shape.*`, `minsert.*`, `addselected.*`). These tests pin
//! the admitted token shapes and prove the selectors are argument selectors.

use crate::session::commands::{
    parse_addselected_argument, parse_minsert_argument, parse_shape_argument, ADDSELECTED_SELECTOR,
    ARGUMENT_SELECTORS, MINSERT_SELECTOR, SHAPE_SELECTOR,
};
use cad_app::{AddSelectedCommand, MInsertCommand};
use cad_domain::Point3;

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn shape_argument_is_a_name_or_code_plus_a_point_and_optional_size_rotation() {
    let by_code = parse_shape_argument("65 10 20").expect("code form");
    assert_eq!(by_code.code, 65);
    assert!(by_code.name.is_empty());
    assert_eq!(by_code.position, point(10.0, 20.0));
    assert_eq!(by_code.size, None);
    assert_eq!(by_code.rotation_degrees, None);

    let by_name = parse_shape_argument("ARROW 0 0 5 45").expect("name form");
    assert_eq!(by_name.code, 0);
    assert_eq!(by_name.name, "ARROW");
    assert_eq!(by_name.size, Some(5.0));
    assert_eq!(by_name.rotation_degrees, Some(45.0));

    let negative = parse_shape_argument("ARROW -2.5 3 1 -30").expect("signed point/rotation");
    assert_eq!(negative.position, point(-2.5, 3.0));
    assert_eq!(negative.size, Some(1.0));
    assert_eq!(negative.rotation_degrees, Some(-30.0));

    for refused in [
        "",
        "65",
        "65 10",
        "65 10 20 5 45 extra",
        // A zero code is not a code (`0` is reserved for the name form) and the
        // parser never repairs it.
        "0 10 20",
        // Size must be a positive plain decimal.
        "65 10 20 0",
        "65 10 20 -1",
        "65 10 20 1e3",
        "65 1e1 20",
        "65 x 20",
    ] {
        assert_eq!(parse_shape_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn minsert_argument_is_a_block_two_counts_two_spacings_and_a_point() {
    assert_eq!(
        parse_minsert_argument("B 2 3 10 -5 1 2"),
        Some(MInsertCommand {
            block: "B".into(),
            rows: 2,
            columns: 3,
            row_spacing: 10.0,
            column_spacing: -5.0,
            insertion: point(1.0, 2.0),
        })
    );
    for refused in [
        "",
        "B 2 3 10 -5 1",
        "B 0 3 10 -5 1 2",
        "B 2 0 10 -5 1 2",
        "B 2.5 3 10 -5 1 2",
        "B -2 3 10 -5 1 2",
        "B 2 3 x -5 1 2",
        "B 2 3 10 -5 1 2 3",
    ] {
        assert_eq!(parse_minsert_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn addselected_argument_is_one_point() {
    assert_eq!(
        parse_addselected_argument("-1.5 2"),
        Some(AddSelectedCommand {
            position: point(-1.5, 2.0)
        })
    );
    for refused in ["", "1", "1 2 3", "x 2", "1 2 extra"] {
        assert_eq!(parse_addselected_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn the_three_selectors_are_admitted_argument_selectors() {
    for selector in [SHAPE_SELECTOR, MINSERT_SELECTOR, ADDSELECTED_SELECTOR] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must take a typed argument"
        );
    }
}
