//! `MLEDIT` / `-MLEDIT` / `MLEADER` / `MLEADERALIGN` / `MLEADEREDIT` /
//! `TOLERANCE` typed-argument grammars.
//!
//! The parsers are pure and strict; the command layer is the authority for every
//! semantic refusal (`mledit.*`, `mleader.*`, `mleaderalign.*`, `mleaderedit.*`,
//! `tolerance.*`). These tests pin the admitted token shapes and prove every
//! selector is an argument selector (`MLEADERCOLLECT` deliberately is not). The
//! `MLEADER` angle constraint takes one of the three stored modes; a numeric
//! rotation and an unknown mode are pinned as *named* refusals, never silent
//! drops.

use crate::session::commands::{
    parse_mleader_argument, parse_mleaderalign_argument, parse_mleaderedit_argument,
    parse_mledit_argument, parse_tolerance_argument, MLeaderCreateInput, MLeaderParseError,
    MLeaderTextAngleError, ARGUMENT_SELECTORS, DASH_MLEDIT_SELECTOR, MLEADERALIGN_SELECTOR,
    MLEADEREDIT_SELECTOR, MLEADER_SELECTOR, MLEDIT_SELECTOR, TOLERANCE_SELECTOR,
};
use cad_app::{
    MLeaderAlignCommand, MLeaderAlignMode, MLeaderAttachment, MLeaderEditAction,
    MLeaderEditCommand, MLeaderLineOverrideField, MlEditCommand, MlEditMode, ToleranceCommand,
};
use cad_domain::{MLeaderContent, MLeaderPathType, MLeaderTextAngle, Point3};

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
        // The per-line override forms are strict too: one index, one value.
        "PATHTYPE",
        "PATHTYPE 0",
        "PATHTYPE 0 CURVE",
        "PATHTYPE 0 SPLINE 1",
        "PATHTYPE -1 SPLINE",
        "ARROWSIZE 0",
        "ARROWSIZE 0 0",
        "ARROWSIZE 0 -1",
        "ARROWSIZE 0 x",
        "BYSTYLE 0",
        "BYSTYLE 0 COLOR",
        "BYSTYLE 0 PATH 1",
    ] {
        assert_eq!(parse_mleaderedit_argument(refused), None, "{refused}");
    }
}

#[test]
fn mleaderedit_argument_sets_and_clears_one_lines_own_overrides() {
    // The three path types map onto the DXF `170` codes; the two clearable fields
    // onto the two `93` override bits this build models.
    for (token, path_type) in [
        ("LINE", MLeaderPathType::StraightLineSegments),
        ("spline", MLeaderPathType::Spline),
        ("Invisible", MLeaderPathType::Invisible),
    ] {
        assert_eq!(
            parse_mleaderedit_argument(&format!("PATHTYPE 2 {token}")),
            Some(MLeaderEditCommand {
                action: MLeaderEditAction::SetPathType {
                    line_index: 2,
                    path_type,
                },
            })
        );
    }
    assert_eq!(
        parse_mleaderedit_argument("ARROWSIZE 0 2.5"),
        Some(MLeaderEditCommand {
            action: MLeaderEditAction::SetArrowheadSize {
                line_index: 0,
                size: 2.5,
            },
        })
    );
    for (token, field) in [
        ("PATH", MLeaderLineOverrideField::PathType),
        ("arrow", MLeaderLineOverrideField::ArrowheadSize),
    ] {
        assert_eq!(
            parse_mleaderedit_argument(&format!("BYSTYLE 1 {token}")),
            Some(MLeaderEditCommand {
                action: MLeaderEditAction::ClearOverride {
                    line_index: 1,
                    field,
                },
            })
        );
    }
}

#[test]
fn mleader_argument_is_the_creation_line_with_an_optional_option_prefix() {
    // The plain form: four coordinates and the text remainder.
    assert_eq!(
        parse_mleader_argument("0 0 10 2 Note"),
        Ok(MLeaderCreateInput {
            arrow: point(0.0, 0.0),
            landing: point(10.0, 2.0),
            text: "Note".into(),
            style: None,
            dogleg_length: None,
            dogleg_angle: None,
            attachment: None,
            text_angle: None,
        })
    );
    // The text is the remainder: internal and trailing spaces survive, the leading
    // separator does not (the same policy `TOLERANCE` documents).
    assert_eq!(
        parse_mleader_argument("0 0 10 2  two  words "),
        Ok(MLeaderCreateInput {
            arrow: point(0.0, 0.0),
            landing: point(10.0, 2.0),
            text: "two  words ".into(),
            style: None,
            dogleg_length: None,
            dogleg_angle: None,
            attachment: None,
            text_angle: None,
        })
    );
    // The option prefix, in any order; `DOGLEGANGLE` is degrees on the line and
    // radians in the payload.
    let parsed = parse_mleader_argument("attach 1 4 DOGLEGANGLE 90 dogleg 2.5 -3 1 7 2 label")
        .expect("the option prefix is admitted");
    assert_eq!(parsed.arrow, point(-3.0, 1.0));
    assert_eq!(parsed.landing, point(7.0, 2.0));
    assert_eq!(parsed.text, "label");
    assert_eq!(parsed.dogleg_length, Some(2.5));
    let angle = parsed.dogleg_angle.expect("DOGLEGANGLE is stored");
    assert!(
        (angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
        "{angle}"
    );
    assert_eq!(
        parsed.attachment,
        Some(MLeaderAttachment { point: 1, left: 4 })
    );

    // The angle constraint takes a **mode** (DXF 174/175 stores an angle type)
    // and both spellings are admitted; a numeric rotation is refused by name.
    assert_eq!(
        parse_mleader_argument("TEXTANGLE PARALLEL 0 0 10 2 Note"),
        Ok(MLeaderCreateInput {
            arrow: point(0.0, 0.0),
            landing: point(10.0, 2.0),
            text: "Note".into(),
            style: None,
            dogleg_length: None,
            dogleg_angle: None,
            attachment: None,
            text_angle: Some(MLeaderTextAngle::ParallelToLastLeaderLine),
        })
    );
    assert_eq!(
        parse_mleader_argument("angle optimized 0 0 10 2 Note")
            .expect("the ANGLE alias")
            .text_angle,
        Some(MLeaderTextAngle::Optimized)
    );
    assert_eq!(
        parse_mleader_argument("TEXTANGLE 90 0 0 10 2 Note"),
        Err(MLeaderParseError::TextAngle(
            MLeaderTextAngleError::RotationAngle("90".to_string())
        ))
    );
    for non_finite in ["nan", "inf", "-inf"] {
        assert_eq!(
            parse_mleader_argument(&format!("TEXTANGLE {non_finite} 0 0 10 2 Note")),
            Err(MLeaderParseError::TextAngle(
                MLeaderTextAngleError::RotationAngle(non_finite.to_string())
            )),
            "{non_finite}"
        );
    }
    assert_eq!(
        parse_mleader_argument("TEXTANGLE diagonal 0 0 10 2 Note"),
        Err(MLeaderParseError::TextAngle(
            MLeaderTextAngleError::UnknownMode("diagonal".to_string())
        ))
    );
    // `STYLE` takes one name token (a single token, like the rest of the line).
    assert_eq!(
        parse_mleader_argument("STYLE Callout 0 0 10 2 Note")
            .expect("the style selector")
            .style
            .as_deref(),
        Some("Callout")
    );
    // A duplicated option is outside the grammar either way.
    assert_eq!(
        parse_mleader_argument("STYLE A STYLE B 0 0 10 2 Note"),
        Err(MLeaderParseError::Invalid)
    );
    assert_eq!(
        parse_mleader_argument("TEXTANGLE PARALLEL ANGLE HORIZONTAL 0 0 10 2 Note"),
        Err(MLeaderParseError::Invalid)
    );

    // A missing text is *not* a grammar error either: it creates a content-less
    // leader, so the parser hands over an empty string.
    assert_eq!(
        parse_mleader_argument("0 0 10 2"),
        Ok(MLeaderCreateInput {
            arrow: point(0.0, 0.0),
            landing: point(10.0, 2.0),
            text: String::new(),
            style: None,
            dogleg_length: None,
            dogleg_angle: None,
            attachment: None,
            text_angle: None,
        })
    );
    for refused in [
        "",
        "0",
        "0 0",
        "0 0 10",
        "DOGLEG",
        "DOGLEG 0 0 0 10 2 Note", // a dogleg length must be positive
        "DOGLEGANGLE x 0 0 10 2 Note",
        "DOGLEG 1 DOGLEG 2 0 0 10 2 Note",
        "ATTACH 1 0 0 10 2 Note", // the pair needs two codes
        "ATTACH x 1 0 0 10 2 Note",
        "Note 0 0 10 2", // the options are a prefix, not a suffix
    ] {
        assert_eq!(
            parse_mleader_argument(refused),
            Err(MLeaderParseError::Invalid),
            "{refused}"
        );
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
        MLEADER_SELECTOR,
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
fn typed_mleader_creates_a_leader_and_refuses_an_unstorable_angle() {
    let session = fresh_session();
    session
        .command("MLEADER 0 0 10 2 Note")
        .expect("the typed MLEADER creates one leader");
    session
        .command("MLEADER DOGLEG 2.5 DOGLEGANGLE 90 ATTACH 1 4 TEXTANGLE PARALLEL -3 1 7 2 Typed")
        .expect("the typed creation options are admitted");
    session
        .command("MLEADER STYLE Standard ANGLE OPTIMIZED 0 0 10 2 Selected")
        .expect("the typed style selector is admitted");
    session
        .command("MLEADER 0 0 10 2")
        .expect("an empty text creates a content-less leader");
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a drawing is open");
    let leaders: Vec<_> = drawing
        .model_space()
        .into_iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(leader) => Some(leader.as_ref()),
            _ => None,
        })
        .collect();
    // The fixture drawing may carry its own entities; the created leaders are
    // identified by their annotation strings.
    let plain = leaders
        .iter()
        .find(|leader| leader.text().string == "Note")
        .expect("the plain typed leader");
    assert!(plain.roots()[0].dogleg_length > 0.0);
    let typed = leaders
        .iter()
        .find(|leader| leader.text().string == "Typed")
        .expect("the leader with the typed options");
    assert_eq!(typed.roots()[0].dogleg_length, 2.5);
    // 90° is `+Y`; `cos(π/2)` leaves a ~6e-17 residue, so compare with tolerance.
    assert!(typed.roots()[0].dogleg_vector.x.abs() < 1e-12);
    assert!((typed.roots()[0].dogleg_vector.y - 1.0).abs() < 1e-12);
    assert_eq!(typed.text().attachment_point, 1);
    assert_eq!(typed.text().left_attachment, 4);
    // The typed mode is the stored snapshot, with the entity override bit.
    assert_eq!(
        typed.style().text_angle_type,
        MLeaderTextAngle::ParallelToLastLeaderLine
    );
    assert_ne!(
        typed.override_mask() & cad_domain::MASK_TEXT_ANGLE,
        0,
        "an entity value that differs from the style's names 0x2000"
    );
    let selected = leaders
        .iter()
        .find(|leader| leader.text().string == "Selected")
        .expect("the leader created from the typed style");
    assert_eq!(selected.style_name(), "Standard");
    assert_eq!(
        selected.style().text_angle_type,
        MLeaderTextAngle::Optimized
    );
    // The empty text is a content-less leader, not a refusal.
    assert!(
        leaders
            .iter()
            .any(|leader| leader.content() == MLeaderContent::None),
        "the typed line without text creates a content-less leader"
    );
    drop(controller);

    // A numeric angle has no storable mode: refused by name, nothing written.
    let before = {
        let controller = session.core.controller.borrow();
        controller.drawing().expect("a drawing is open").revision()
    };
    for line in [
        "MLEADER TEXTANGLE 2 0 0 10 2 Note",
        "MLEADER TEXTANGLE nan 0 0 10 2 Note",
        "MLEADER TEXTANGLE diagonal 0 0 10 2 Note",
    ] {
        let error = session
            .command(line)
            .expect_err("an unstorable angle is refused by name");
        assert!(
            error.contains("mleader.option_text_angle"),
            "{line}: {error}"
        );
        assert!(error.contains("174"), "{line}: {error}");
    }
    // An unknown style name is refused by name too.
    let error = session
        .command("MLEADER STYLE NoSuchStyle 0 0 10 2 Note")
        .expect_err("an unknown style is refused");
    assert!(error.contains("mleader.style_missing"), "{error}");
    let after = {
        let controller = session.core.controller.borrow();
        controller.drawing().expect("a drawing is open").revision()
    };
    assert_eq!(before, after, "the refusals wrote nothing");
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
