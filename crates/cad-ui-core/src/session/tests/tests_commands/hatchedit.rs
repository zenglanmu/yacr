//! `HATCHEDIT` (AutoCAD 2026) vocabulary: the typed v1 option subset.
//!
//! Pins the argument grammar (`ANGLE <degrees>`, `SCALE <factor>`, `SOLID`,
//! `GRADIENT`), the stable invalid-argument refusal, the Work-only capability
//! and the one-canonical-selector rule (the option words are grammar, not
//! names).

use super::super::super::*;
use super::super::test_support::*;

#[test]
fn hatchedit_is_the_declared_work_only_typed_subset() {
    for (argument, option) in [
        ("SOLID", cad_app::HatchEditOption::Solid),
        ("solid", cad_app::HatchEditOption::Solid),
        ("GRADIENT", cad_app::HatchEditOption::Gradient),
        ("gradient", cad_app::HatchEditOption::Gradient),
        ("ANGLE 45", cad_app::HatchEditOption::Angle(45.0)),
        ("angle -22.5", cad_app::HatchEditOption::Angle(-22.5)),
        ("SCALE 2", cad_app::HatchEditOption::Scale(2.0)),
        ("scale 0.5", cad_app::HatchEditOption::Scale(0.5)),
    ] {
        assert_eq!(
            parse_hatchedit_argument(argument),
            Some(option),
            "{argument}"
        );
    }
    // No other AutoCAD dialog field, option stack, missing value, zero/negative
    // scale, exponent or garbage.
    for refused in [
        "",
        "PATTERN",
        "COLOR",
        "PROPERTIES",
        "DRAWORDER",
        "ADD",
        "REMOVE",
        "ASSOCIATIVE",
        "ANGLE",
        "SCALE",
        "ANGLE ",
        "SCALE 0",
        "SCALE -1",
        "ANGLE 1e3",
        "SOLID 1",
        "GRADIENT X",
    ] {
        assert_eq!(parse_hatchedit_argument(refused), None, "{refused:?}");
    }
    // The refusal names the implemented forms and the unimplemented dialog fields.
    let reason = invalid_hatchedit_argument_reason("HATCHEDIT PATTERN");
    assert!(reason.contains("ANGLE <degrees>"), "{reason}");
    assert!(reason.contains("SCALE <factor>"), "{reason}");
    assert!(reason.contains("SOLID"), "{reason}");
    assert!(reason.contains("GRADIENT"), "{reason}");
    assert!(reason.contains("NOT implemented"), "{reason}");
    // It is a typed argument of one canonical selector, not a new command name.
    assert_eq!(
        typed_command_argument("HATCHEDIT ANGLE 45"),
        Some(("HATCHEDIT", "ANGLE 45"))
    );
    assert_eq!(
        typed_command_argument("HATCHEDIT SOLID"),
        Some(("HATCHEDIT", "SOLID"))
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
