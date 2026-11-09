//! `HATCHEDIT` (AutoCAD 2026) vocabulary: the typed option subset.
//!
//! Pins the argument grammar (`STYLE`, `ANGLE`, `SCALE`, `PATTERNTYPE`, `SOLID`,
//! `GRADIENT`, `COLOR`, `TRANSPARENCY`, `LAYER`, `DRAWORDER`,
//! `RECREATEBOUNDARY`, `SEPARATE` plus the named-refusal forms), the stable
//! invalid-argument refusal, the Work-only capability and the one-canonical-
//! selector rule (the option words are grammar, not names).

use super::super::super::*;
use super::super::test_support::*;

use cad_app::{HatchDrawOrder, HatchEditOption};
use cad_domain::{HatchPatternType, HatchStyle};

#[test]
fn hatchedit_is_the_declared_work_only_typed_subset() {
    for (argument, option) in [
        ("SOLID", HatchEditOption::Solid),
        ("solid", HatchEditOption::Solid),
        ("GRADIENT", HatchEditOption::Gradient),
        ("gradient", HatchEditOption::Gradient),
        ("ANGLE 45", HatchEditOption::Angle(45.0)),
        ("angle -22.5", HatchEditOption::Angle(-22.5)),
        ("SCALE 2", HatchEditOption::Scale(2.0)),
        ("scale 0.5", HatchEditOption::Scale(0.5)),
        ("STYLE OUTER", HatchEditOption::Style(HatchStyle::Outer)),
        ("style ignore", HatchEditOption::Style(HatchStyle::Ignore)),
        (
            "PATTERNTYPE PREDEFINED",
            HatchEditOption::PatternType(HatchPatternType::Predefined),
        ),
        (
            "DRAWORDER BEHIND",
            HatchEditOption::DrawOrder(HatchDrawOrder::BehindAll),
        ),
        (
            "DRAWORDER FRONT",
            HatchEditOption::DrawOrder(HatchDrawOrder::InFrontOfAll),
        ),
        (
            "DRAWORDER BOUNDARY",
            HatchEditOption::DrawOrder(HatchDrawOrder::BehindBoundary),
        ),
        ("RECREATEBOUNDARY", HatchEditOption::RecreateBoundary),
        ("SEPARATE", HatchEditOption::SeparateHatches),
        ("COLOR 30", HatchEditOption::Color(30)),
        ("COLOR BYLAYER", HatchEditOption::ColorByLayer),
        ("TRANSPARENCY 50", HatchEditOption::Transparency(50)),
        ("TRANSPARENCY BYLAYER", HatchEditOption::TransparencyByLayer),
        ("LAYER Walls", HatchEditOption::Layer("Walls".into())),
        ("ORIGIN", HatchEditOption::Origin),
        ("ASSOCIATE", HatchEditOption::Associate),
        ("DISASSOCIATE", HatchEditOption::Disassociate),
        ("ANNOTATIVE", HatchEditOption::Annotative),
        ("ADDBOUNDARIES", HatchEditOption::AddBoundaries),
        ("REMOVEBOUNDARIES", HatchEditOption::RemoveBoundaries),
    ] {
        assert_eq!(
            parse_hatchedit_argument(argument),
            Some(option),
            "{argument}"
        );
    }
    // No other AutoCAD dialog field, option stack, missing value, zero/negative
    // scale, out-of-range colour/transparency, exponent or garbage.
    for refused in [
        "",
        "PATTERN",
        "PROPERTIES",
        "COLOR",
        "COLOR 0",
        "COLOR 256",
        "TRANSPARENCY",
        "TRANSPARENCY 91",
        "DRAWORDER",
        "DRAWORDER SIDEWAYS",
        "ADD",
        "REMOVE",
        "ASSOCIATIVE",
        "ANGLE",
        "SCALE",
        "ANGLE ",
        "SCALE 0",
        "SCALE -1",
        "ANGLE 1e3",
        "STYLE",
        "STYLE SIDEWAYS",
        "PATTERNTYPE",
        "LAYER",
        "SOLID 1",
        "GRADIENT X",
    ] {
        assert_eq!(parse_hatchedit_argument(refused), None, "{refused:?}");
    }
    // The refusal names the implemented forms and the named refusals.
    let reason = invalid_hatchedit_argument_reason("HATCHEDIT PATTERN");
    assert!(reason.contains("STYLE"), "{reason}");
    assert!(reason.contains("COLOR"), "{reason}");
    assert!(reason.contains("RECREATEBOUNDARY"), "{reason}");
    assert!(reason.contains("named refusals"), "{reason}");
    // It is a typed argument of one canonical selector, not a new command name.
    assert_eq!(
        typed_command_argument("HATCHEDIT ANGLE 45"),
        Some(("HATCHEDIT", "ANGLE 45"))
    );
    assert_eq!(
        typed_command_argument("HATCHEDIT STYLE OUTER"),
        Some(("HATCHEDIT", "STYLE OUTER"))
    );
    assert_eq!(
        typed_command_argument("HATCHEDIT"),
        None,
        "bare HATCHEDIT is name-only"
    );
    assert!(CommandId::HatchEdit.requires_work_mode());
    assert_eq!(command_name(CommandId::HatchEdit), Some("HATCHEDIT"));

    // A bare selector is admitted by name and refused for its missing option; a
    // bad argument never reaches the command layer; a good one reaches it and
    // reports the absent selection (there is no drawing in this fixture).
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let error = session.command("HATCHEDIT").unwrap_err();
    assert!(error.contains("hatchedit.needs_option"), "{error}");
    let error = session.command("HATCHEDIT PATTERN").unwrap_err();
    assert!(error.contains("HATCHEDIT needs"), "{error}");
    let error = session.command("HATCHEDIT ANGLE 45").unwrap_err();
    assert!(error.contains("hatchedit.selection"), "{error}");
    let error = session.command("HATCHEDIT SCALE 0").unwrap_err();
    assert!(error.contains("HATCHEDIT needs"), "{error}");
}
