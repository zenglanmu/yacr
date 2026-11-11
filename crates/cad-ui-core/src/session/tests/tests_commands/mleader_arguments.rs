//! `MLEDIT` / `-MLEDIT` / `MLEADERALIGN` / `MLEADEREDIT` / `TOLERANCE`
//! typed-argument grammars.
//!
//! The parsers are pure and strict; the command layer is the authority for every
//! semantic refusal (`mledit.*`, `mleaderalign.*`, `mleaderedit.*`,
//! `tolerance.*`). These tests pin the admitted token shapes and prove every
//! selector is an argument selector (`MLEADERCOLLECT` deliberately is not).

use crate::session::commands::{
    parse_mleaderalign_argument, parse_mleaderedit_argument, parse_mledit_argument,
    parse_tolerance_argument, ARGUMENT_SELECTORS, DASH_MLEDIT_SELECTOR, MLEADERALIGN_SELECTOR,
    MLEADEREDIT_SELECTOR, MLEDIT_SELECTOR, TOLERANCE_SELECTOR,
};
use cad_app::{
    MLeaderAlignCommand, MLeaderAlignMode, MLeaderEditAction, MLeaderEditCommand, MlEditCommand,
    MlEditMode, ToleranceCommand,
};
use cad_domain::Point3;

use super::super::super::*;
use super::super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn mledit_argument_is_an_option_plus_the_pick_or_the_band() {
    assert_eq!(
        parse_mledit_argument("MERGE 2 3"),
        Some(MlEditCommand {
            mode: MlEditMode::Merge,
            first: point(2.0, 3.0),
            second: None,
        })
    );
    for (word, mode) in [
        ("CLOSED", MlEditMode::Closed),
        ("CROSS", MlEditMode::Cross),
        ("TEE", MlEditMode::Tee),
        ("OPEN", MlEditMode::Open),
    ] {
        let parsed = parse_mledit_argument(&format!("{word} -1.5 4")).expect("cleanup/open form");
        assert_eq!(parsed.mode, mode);
        assert_eq!(parsed.first, point(-1.5, 4.0));
        assert_eq!(parsed.second, None);
    }
    assert_eq!(
        parse_mledit_argument("BREAK 0 0 10 2"),
        Some(MlEditCommand {
            mode: MlEditMode::Break,
            first: point(0.0, 0.0),
            second: Some(point(10.0, 2.0)),
        })
    );
    // The grammar is strict: the wrong coordinate count for the option, an
    // unknown option, an exponent or a missing token is refused.
    for refused in [
        "",
        "MERGE",
        "MERGE 2",
        "MERGE 2 3 4",
        "BREAK 0 0 1",
        "BREAK 0 0 1 1 2",
        "OPEN 1e3 0",
        "ZOOM 1 2",
    ] {
        assert_eq!(parse_mledit_argument(refused), None, "{refused}");
    }
}

#[test]
fn mleaderalign_argument_is_a_mode_and_an_optional_positive_spacing() {
    assert_eq!(
        parse_mleaderalign_argument("LEFT"),
        Some(MLeaderAlignCommand {
            mode: MLeaderAlignMode::Left,
            spacing: None,
        })
    );
    assert_eq!(
        parse_mleaderalign_argument("right 2.5"),
        Some(MLeaderAlignCommand {
            mode: MLeaderAlignMode::Right,
            spacing: Some(2.5),
        })
    );
    assert_eq!(
        parse_mleaderalign_argument("C"),
        Some(MLeaderAlignCommand {
            mode: MLeaderAlignMode::Center,
            spacing: None,
        })
    );
    for refused in [
        "", "MIDDLE", "LEFT 0", "LEFT -1", "LEFT 2 3", "LEFT NaN", "LEFT 1e2",
    ] {
        assert_eq!(parse_mleaderalign_argument(refused), None, "{refused}");
    }
}

#[test]
fn mleaderedit_argument_adds_at_a_point_or_removes_a_zero_based_index() {
    assert_eq!(
        parse_mleaderedit_argument("ADD 5 -2"),
        Some(MLeaderEditCommand {
            action: MLeaderEditAction::Add {
                arrow: point(5.0, -2.0)
            },
        })
    );
    assert_eq!(
        parse_mleaderedit_argument("REMOVE 1"),
        Some(MLeaderEditCommand {
            action: MLeaderEditAction::Remove { line_index: 1 },
        })
    );
    for refused in [
        "",
        "ADD",
        "ADD 1",
        "ADD 1 2 3",
        "REMOVE",
        "REMOVE -1",
        "REMOVE 1.0",
        "REMOVE 1 2",
        "DELETE 1",
    ] {
        assert_eq!(parse_mleaderedit_argument(refused), None, "{refused}");
    }
}

#[test]
fn tolerance_argument_is_a_corner_and_the_text_remainder() {
    assert_eq!(
        parse_tolerance_argument("1.5 -2 |0.05|A"),
        Some(ToleranceCommand {
            origin: point(1.5, -2.0),
            text: "|0.05|A".into(),
        })
    );
    // The remainder is kept verbatim (internal spaces preserved); a missing text
    // is handed to the command layer so its own `tolerance.text` refusal answers.
    assert_eq!(
        parse_tolerance_argument("0 0  flatness  0.1 "),
        Some(ToleranceCommand {
            origin: point(0.0, 0.0),
            text: "flatness  0.1 ".into(),
        })
    );
    assert_eq!(
        parse_tolerance_argument("0 0"),
        Some(ToleranceCommand {
            origin: point(0.0, 0.0),
            text: String::new(),
        })
    );
    for refused in ["", "0", "x 0 A", "0 y A"] {
        assert_eq!(parse_tolerance_argument(refused), None, "{refused}");
    }
}

#[test]
fn every_typed_selector_is_an_argument_selector_and_the_bare_forms_are_not() {
    for selector in [
        MLEDIT_SELECTOR,
        DASH_MLEDIT_SELECTOR,
        MLEADERALIGN_SELECTOR,
        MLEADEREDIT_SELECTOR,
        TOLERANCE_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be admitted with an argument"
        );
    }
    // MLEADERCOLLECT takes no argument: it must stay out of the argument table so
    // its bare selector keeps the ordinary command path.
    assert!(!ARGUMENT_SELECTORS.contains(&"MLEADERCOLLECT"));
}

#[test]
fn typed_tolerance_dispatches_a_real_frame() {
    let session = fresh_session();
    session
        .command("TOLERANCE 100 200 0.05|A")
        .expect("the typed TOLERANCE builds one frame");
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a drawing is open");
    let entities: Vec<_> = drawing.model_space().into_iter().cloned().collect();
    // The fixture drawing may carry its own entities; the frame is identified by
    // its lower-left corner.
    let borders: Vec<_> = entities
        .iter()
        .filter(|entity| match &entity.geometry {
            SemanticGeometry::Polyline { points, closed, .. } => {
                *closed
                    && points.len() == 4
                    && (points[0].x - 100.0).abs() < 1e-9
                    && (points[0].y - 200.0).abs() < 1e-9
            }
            _ => false,
        })
        .collect();
    assert_eq!(borders.len(), 1, "one border polyline");
    let texts: Vec<&str> = entities
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { text, .. } if text == "0.05" || text == "A" => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 2, "one TEXT per compartment");
}

#[test]
fn typed_selectors_reach_the_command_layer_with_named_refusals() {
    let session = fresh_session();
    let error = session
        .command("MLEADERALIGN LEFT")
        .expect_err("no multileader is selected");
    assert!(error.contains("mleaderalign.selection"), "{error}");

    let error = session
        .command("-MLEDIT CROSS 0 0")
        .expect_err("no two lines are selected");
    assert!(error.contains("mledit.selection"), "{error}");

    let error = session
        .command("MLEADEREDIT ADD 0 0")
        .expect_err("no multileader is selected");
    assert!(error.contains("mleaderedit.selection"), "{error}");

    let error = session
        .command("TOLERANCE 0 0")
        .expect_err("empty frame text is refused by the command layer");
    assert!(error.contains("tolerance.text"), "{error}");
}
