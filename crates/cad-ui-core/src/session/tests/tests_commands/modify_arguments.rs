//! The typed option lines of the interactive modify commands:
//! `PEDIT` (full line), `JOIN`, `OFFSET`, `MIRROR`, `CHAMFER`, `FILLET`,
//! plus the structured `QSELECT` / `FILTER` grammars.
//!
//! Every form is an **argument of its canonical selector** (never a new name)
//! and every malformed/unknown argument is refused by a stable reason. A bare
//! selector keeps its previous payload behaviour: `PEDIT` → `pedit.needs_option`,
//! `JOIN` → source-object/no-close, `MIRROR`/`OFFSET` → the interactive capture,
//! and the legacy `PROPERTY VALUE` filter line still works.

use super::super::super::commands::{
    invalid_chamfer_argument_reason, invalid_fillet_argument_reason,
    invalid_filter_logic_argument_reason, invalid_join_argument_reason,
    invalid_mirror_argument_reason, invalid_offset_argument_reason,
    invalid_pedit_edit_argument_reason, invalid_qselect_argument_reason, parse_chamfer_argument,
    parse_fillet_argument, parse_filter_request, parse_join_argument, parse_mirror_argument,
    parse_offset_argument, parse_pedit_argument, parse_pedit_edit_argument, parse_qselect_request,
    ChamferArgument, FilletArgument, MirrorArgument, OffsetArgument,
};
use super::super::super::*;
use super::super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn corner_modes(session: &CadSession) -> cad_app::ChamferMethod {
    session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .corner_modes()
        .chamfer_method()
}

#[test]
fn pedit_full_option_line_routes_the_edit_payload() {
    use cad_app::{PeditEditCommand, PeditEditOption, PeditVertexAction, PeditVertexCommand};

    for (argument, option) in [
        ("JOIN", PeditEditOption::Join),
        ("J", PeditEditOption::Join),
        ("EDIT", PeditEditOption::EditVertex),
        (
            "EDIT NEXT 2",
            PeditEditOption::Vertex(PeditVertexCommand {
                action: PeditVertexAction::Next { from: 2 },
            }),
        ),
        (
            "EDIT PREVIOUS 1",
            PeditEditOption::Vertex(PeditVertexCommand {
                action: PeditVertexAction::Previous { from: 1 },
            }),
        ),
        (
            "EDIT MOVE 1 2.5 -3",
            PeditEditOption::Vertex(PeditVertexCommand {
                action: PeditVertexAction::Move {
                    vertex: 1,
                    point: cad_domain::Point3 {
                        x: 2.5,
                        y: -3.0,
                        z: 0.0,
                    },
                },
            }),
        ),
        (
            "EDIT INSERT 0 4 1 2",
            PeditEditOption::Vertex(PeditVertexCommand {
                action: PeditVertexAction::Insert {
                    after: 0,
                    point: cad_domain::Point3 {
                        x: 4.0,
                        y: 1.0,
                        z: 2.0,
                    },
                },
            }),
        ),
        (
            "EDIT STRAIGHTEN 0 3",
            PeditEditOption::Vertex(PeditVertexCommand {
                action: PeditVertexAction::Straighten { first: 0, last: 3 },
            }),
        ),
        (
            "EDIT TANGENT 2 45",
            PeditEditOption::Vertex(PeditVertexCommand {
                action: PeditVertexAction::Tangent {
                    vertex: 2,
                    angle_degrees: 45.0,
                },
            }),
        ),
        (
            "EDIT WIDTH",
            PeditEditOption::Vertex(PeditVertexCommand {
                action: PeditVertexAction::Width,
            }),
        ),
        ("FIT", PeditEditOption::Fit),
        ("DECURVE", PeditEditOption::Decurve),
        ("UNDO", PeditEditOption::Undo),
        ("WIDTH 1.5", PeditEditOption::Width(1.5)),
        ("SPLINE 32", PeditEditOption::Spline { precision: 32 }),
        ("SPLINE", PeditEditOption::Spline { precision: 8 }),
        ("LTYPE GEN", PeditEditOption::LtypeGen { enabled: true }),
        ("LTYPE OFF", PeditEditOption::LtypeGen { enabled: false }),
    ] {
        assert_eq!(
            parse_pedit_edit_argument(argument),
            Some(PeditEditCommand {
                option,
                convert: false
            }),
            "{argument}"
        );
    }
    // The PEDITACCEPT answer prefixes the option in both documented spellings.
    assert_eq!(
        parse_pedit_edit_argument("CONVERT Y JOIN"),
        Some(PeditEditCommand {
            option: PeditEditOption::Join,
            convert: true
        })
    );
    assert_eq!(
        parse_pedit_edit_argument("N FIT"),
        Some(PeditEditCommand {
            option: PeditEditOption::Fit,
            convert: false
        })
    );
    // An answer alone has no option to run.
    assert_eq!(parse_pedit_edit_argument("Y"), None);
    assert_eq!(parse_pedit_edit_argument("CONVERT Y"), None);
    for refused in [
        "SPLINE 100",
        "SPLINE 3.5",
        "WIDTH",
        "LTYPE MAYBE",
        "CLOSE OPEN",
        // A malformed `Edit vertex` sub-menu is refused, never defaulted.
        "EDIT NEXT",
        "EDIT NEXT x",
        "EDIT MOVE 1 2",
        "EDIT INSERT 0 1 2 3 4",
        "EDIT STRAIGHTEN 0",
        "EDIT TANGENT 1",
        "EDIT BOGUS",
    ] {
        assert_eq!(parse_pedit_edit_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_pedit_edit_argument_reason("PEDIT SPLINE 100");
    assert!(reason.contains("SPLINE [0-99]"), "{reason}");

    // The legacy three keep `PeditOption`; every other line reaches `PeditEdit`
    // (the absent selection is the application's named refusal).
    assert_eq!(
        parse_pedit_argument("CLOSE"),
        Some(cad_app::PeditOption::Close)
    );
    let session = fresh_session();
    let error = session.command("PEDIT JOIN").unwrap_err();
    assert!(error.contains("pedit"), "{error}");
    let error = session.command("PEDIT CONVERT Y JOIN").unwrap_err();
    assert!(error.contains("pedit"), "{error}");
    let error = session.command("PEDIT Y").unwrap_err();
    assert!(error.contains("SPLINE [0-99]"), "{error}");
    // Bare PEDIT keeps today's behaviour exactly.
    assert_eq!(typed_command_argument("PEDIT"), None);
    let error = session.command("PEDIT").unwrap_err();
    assert!(error.contains("pedit.needs_option"), "{error}");
}

#[test]
fn join_options_parse_and_bare_join_keeps_source_object_mode() {
    use cad_app::{JoinMode, JoinOptions};

    assert_eq!(
        parse_join_argument("MULTIPLE"),
        Some(JoinOptions {
            close: false,
            mode: JoinMode::Multiple
        })
    );
    assert_eq!(
        parse_join_argument("close"),
        Some(JoinOptions {
            close: true,
            mode: JoinMode::SourceObject
        })
    );
    assert_eq!(
        parse_join_argument("CLOSE MULTIPLE"),
        Some(JoinOptions {
            close: true,
            mode: JoinMode::Multiple
        })
    );
    for refused in ["MULTIPLE MULTIPLE", "CLOSE CLOSE", "ALL", "C M CLOSE"] {
        assert_eq!(parse_join_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_join_argument_reason("JOIN ALL");
    assert!(reason.contains("MULTIPLE and/or CLOSE"), "{reason}");
    assert_eq!(
        typed_command_argument("JOIN MULTIPLE"),
        Some(("JOIN", "MULTIPLE"))
    );
    assert_eq!(typed_command_argument("JOIN"), None);
    let session = fresh_session();
    let error = session.command("JOIN MULTIPLE").unwrap_err();
    assert!(error.contains("join.needs_two"), "{error}");
    // A bare JOIN keeps the source-object, no-close path (same refusal here).
    let error = session.command("JOIN").unwrap_err();
    assert!(error.contains("join.needs_two"), "{error}");
}

#[test]
fn offset_options_parse_and_store_the_session_defaults() {
    assert_eq!(
        parse_offset_argument("2.5"),
        Some(OffsetArgument::Distance(2.5))
    );
    assert_eq!(
        parse_offset_argument("DISTANCE 2.5"),
        Some(OffsetArgument::Distance(2.5))
    );
    assert_eq!(
        parse_offset_argument("THROUGH"),
        Some(OffsetArgument::Through)
    );
    assert_eq!(parse_offset_argument("T"), Some(OffsetArgument::Through));
    assert_eq!(
        parse_offset_argument("ERASE"),
        Some(OffsetArgument::ToggleErase)
    );
    assert_eq!(
        parse_offset_argument("LAYER CURRENT"),
        Some(OffsetArgument::Layer { current: true })
    );
    assert_eq!(
        parse_offset_argument("L SOURCE"),
        Some(OffsetArgument::Layer { current: false })
    );
    assert_eq!(
        parse_offset_argument("MULTIPLE"),
        Some(OffsetArgument::Multiple)
    );
    assert_eq!(parse_offset_argument("U"), Some(OffsetArgument::Undo));
    for refused in ["0", "-2", "LAYER", "LAYER OTHER", "THROUGH 2", "1 2"] {
        assert_eq!(parse_offset_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_offset_argument_reason("OFFSET LAYER OTHER");
    assert!(reason.contains("LAYER CURRENT|SOURCE"), "{reason}");
    assert_eq!(
        typed_command_argument("OFFSET 2.5"),
        Some(("OFFSET", "2.5"))
    );

    let session = fresh_session();
    let options = || session.core.controller.borrow().session.modify_options;
    assert!(options().offset_is_default());
    session.command("OFFSET 2.5").expect("distance stores");
    assert_eq!(options().offset_distance(), Some(2.5));
    session.command("OFFSET THROUGH").expect("Through stores");
    assert_eq!(options().offset_distance(), None);
    session.command("OFFSET ERASE").expect("erase toggles");
    assert!(options().offset_erase());
    session.command("OFFSET ERASE").expect("erase toggles back");
    assert!(!options().offset_erase());
    session
        .command("OFFSET LAYER CURRENT")
        .expect("layer stores");
    assert!(options().offset_current_layer());
    session
        .command("OFFSET LAYER SOURCE")
        .expect("layer stores");
    assert!(!options().offset_current_layer());

    // Multiple is refused by name: the fixed-step capture takes one side point.
    let error = session.command("OFFSET MULTIPLE").unwrap_err();
    assert!(error.contains("offset.multiple_not_interactive"), "{error}");
    // Bare OFFSET keeps the interactive capture (no typed payload).
    assert_eq!(typed_command_argument("OFFSET"), None);
}

#[test]
fn mirror_options_parse_and_store_the_session_defaults() {
    assert_eq!(
        parse_mirror_argument("TEXT 1"),
        Some(MirrorArgument::MirrorText(true))
    );
    assert_eq!(
        parse_mirror_argument("mirrortext 0"),
        Some(MirrorArgument::MirrorText(false))
    );
    assert_eq!(
        parse_mirror_argument("ERASE YES"),
        Some(MirrorArgument::Erase(true))
    );
    assert_eq!(
        parse_mirror_argument("ERASE NO"),
        Some(MirrorArgument::Erase(false))
    );
    for refused in ["TEXT", "TEXT 2", "ERASE MAYBE", "YES", "TEXT 1 EXTRA"] {
        assert_eq!(parse_mirror_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_mirror_argument_reason("MIRROR TEXT 2");
    assert!(reason.contains("TEXT 0|1"), "{reason}");
    assert_eq!(
        typed_command_argument("MIRROR TEXT 1"),
        Some(("MIRROR", "TEXT 1"))
    );
    assert_eq!(typed_command_argument("MIRROR"), None);

    let session = fresh_session();
    let options = || session.core.controller.borrow().session.modify_options;
    assert!(options().mirror_is_default());
    session.command("MIRROR TEXT 1").expect("MIRRTEXT stores");
    assert!(options().mirrortext());
    session.command("MIRROR ERASE YES").expect("erase stores");
    assert!(options().mirror_erase());
    session.command("MIRROR TEXT 0").expect("MIRRTEXT stores");
    assert!(!options().mirrortext());
}

#[test]
fn chamfer_options_route_to_the_stored_modes_and_polyline_form() {
    use cad_app::ChamferMethod;

    assert_eq!(
        parse_chamfer_argument("D 2 3"),
        Some(ChamferArgument::Distances {
            first: 2.0,
            second: Some(3.0)
        })
    );
    assert_eq!(
        parse_chamfer_argument("DISTANCE 2"),
        Some(ChamferArgument::Distances {
            first: 2.0,
            second: None
        })
    );
    assert_eq!(
        parse_chamfer_argument("A 2 45"),
        Some(ChamferArgument::Angle {
            distance: 2.0,
            degrees: 45.0
        })
    );
    assert_eq!(
        parse_chamfer_argument("TRIM"),
        Some(ChamferArgument::Trim(true))
    );
    assert_eq!(
        parse_chamfer_argument("N"),
        Some(ChamferArgument::Trim(false))
    );
    assert_eq!(
        parse_chamfer_argument("METHOD ANGLE"),
        Some(ChamferArgument::Method(ChamferMethod::Angle))
    );
    assert_eq!(
        parse_chamfer_argument("POLYLINE"),
        Some(ChamferArgument::Polyline)
    );
    for refused in ["D", "D 1 2 3", "A 1", "METHOD SPLINE", "P 1", "42"] {
        assert_eq!(parse_chamfer_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_chamfer_argument_reason("CHAMFER D");
    assert!(reason.contains("DISTANCE <d1> [d2]"), "{reason}");

    let session = fresh_session();
    session.command("CHAMFER D 2 3").expect("distances store");
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .tool_parameter(ToolParameter::ChamferDistanceA),
        ToolValue::Number(2.0)
    );
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .tool_parameter(ToolParameter::ChamferDistanceB),
        ToolValue::Number(3.0)
    );
    session.command("CHAMFER N").expect("TRIMMODE stores");
    assert!(!session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .corner_modes()
        .trim());
    session
        .command("CHAMFER A 2 45")
        .expect("angle method stores");
    assert_eq!(corner_modes(&session), ChamferMethod::Angle);
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .corner_modes()
            .chamfer_angle_degrees(),
        45.0
    );
    // The polyline form is a real command dispatch (one selected polyline is
    // the input; the fresh fixture has none, so it is refused by the command).
    let error = session.command("CHAMFER POLYLINE").unwrap_err();
    assert!(!error.contains("CHAMFER needs"), "{error}");
    // `CHAMFER POLYLINE` is still the canonical name-only selector (the whole
    // line resolves before the argument splitter), so the splitter must not
    // reinterpret it as `CHAMFER` + `POLYLINE`; `CHAMFER P` is the typed spelling
    // of the same polyline mode.
    assert_eq!(typed_command_argument("CHAMFER POLYLINE"), None);
    assert_eq!(typed_command_argument("CHAMFER P"), Some(("CHAMFER", "P")));
    assert_eq!(
        typed_command_argument("CHAMFER D 2 3"),
        Some(("CHAMFER", "D 2 3"))
    );
}

#[test]
fn fillet_options_route_to_the_stored_modes_and_polyline_form() {
    assert_eq!(
        parse_fillet_argument("R 2.5"),
        Some(FilletArgument::Radius(2.5))
    );
    assert_eq!(
        parse_fillet_argument("RADIUS 2.5"),
        Some(FilletArgument::Radius(2.5))
    );
    assert_eq!(
        parse_fillet_argument("TRIM"),
        Some(FilletArgument::Trim(true))
    );
    assert_eq!(
        parse_fillet_argument("NOTRIM"),
        Some(FilletArgument::Trim(false))
    );
    assert_eq!(
        parse_fillet_argument("POLYLINE"),
        Some(FilletArgument::Polyline)
    );
    for refused in ["R", "R 1 2", "T 1", "P 1", "2.5"] {
        assert_eq!(parse_fillet_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_fillet_argument_reason("FILLET R");
    assert!(reason.contains("RADIUS <r>"), "{reason}");

    let session = fresh_session();
    session.command("FILLET R 2.5").expect("radius stores");
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .tool_parameter(ToolParameter::FilletRadius),
        ToolValue::Number(2.5)
    );
    session.command("FILLET NOTRIM").expect("TRIMMODE stores");
    assert!(!session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .corner_modes()
        .trim());
    let error = session.command("FILLET POLYLINE").unwrap_err();
    assert!(!error.contains("FILLET needs"), "{error}");
    assert_eq!(typed_command_argument("FILLET POLYLINE"), None);
    assert_eq!(typed_command_argument("FILLET P"), Some(("FILLET", "P")));
    assert_eq!(
        typed_command_argument("FILLET R 2.5"),
        Some(("FILLET", "R 2.5"))
    );
}

#[test]
fn qselect_structured_request_is_reachable_and_legacy_line_still_works() {
    use cad_app::{SelectionApply, SelectionOperator, SelectionProperty, SelectionScope};

    let request = parse_qselect_request("LAYER = WALLS APPEND SELECTION").expect("request");
    assert_eq!(request.predicate.property, SelectionProperty::Layer);
    assert_eq!(request.predicate.operator, SelectionOperator::Equals);
    assert_eq!(request.predicate.value, "WALLS");
    assert_eq!(request.apply, SelectionApply::Append);
    assert_eq!(request.scope, SelectionScope::CurrentSelection);
    assert_eq!(request.object_type, None);

    let request = parse_qselect_request("LINEWEIGHT >= 25 EXCLUDE").expect("request");
    assert_eq!(
        request.predicate.operator,
        SelectionOperator::GreaterThanOrEqual
    );
    assert_eq!(request.apply, SelectionApply::ExcludeNew);
    assert_eq!(request.scope, SelectionScope::EntireDrawing);

    let request = parse_qselect_request("TEXT ~ Wall*").expect("wildcard");
    assert_eq!(request.predicate.operator, SelectionOperator::WildcardMatch);

    // No operator is the legacy `PROPERTY VALUE` form, which stays reachable.
    assert_eq!(parse_qselect_request("LAYER WALLS"), None);
    // Malformed structured lines are refused by name (not downgraded).
    let reason = invalid_qselect_argument_reason("QSELECT LAYER = WALLS APPEND APPEND");
    assert!(reason.contains("APPEND|EXCLUDE"), "{reason}");

    let session = fresh_session();
    session
        .command("QSELECT TYPE = LINE")
        .expect("structured query runs");
    session
        .command("QSELECT LAYER WALLS")
        .expect("legacy query runs");
    let error = session
        .command("QSELECT LAYER = WALLS APPEND APPEND")
        .unwrap_err();
    assert!(error.contains("APPEND|EXCLUDE"), "{error}");
    // A grammar-valid relative comparison on a non-numeric property is the
    // application's named refusal, never a silent empty success.
    let error = session.command("QSELECT TYPE >= LINE").unwrap_err();
    assert!(error.contains("relative_non_numeric"), "{error}");
}

#[test]
fn filter_logic_grammar_is_reachable_with_parenthesised_groups() {
    use cad_app::{FilterExpression, FilterLogic, SelectionOperator, SelectionProperty};

    let request = parse_filter_request("AND LAYER = WALLS TYPE = LINE").expect("and");
    match request.expression {
        FilterExpression::Group { logic, members } => {
            assert_eq!(logic, FilterLogic::And);
            assert_eq!(members.len(), 2);
            match &members[0] {
                FilterExpression::Predicate(predicate) => {
                    assert_eq!(predicate.property, SelectionProperty::Layer);
                    assert_eq!(predicate.operator, SelectionOperator::Equals);
                    assert_eq!(predicate.value, "WALLS");
                }
                other => panic!("expected a predicate, got {other:?}"),
            }
        }
        other => panic!("expected a group, got {other:?}"),
    }

    let request =
        parse_filter_request("AND (OR TYPE = LINE TYPE = ARC) LAYER = WALLS").expect("nested");
    let FilterExpression::Group { members, .. } = request.expression else {
        panic!("expected a group");
    };
    assert!(matches!(&members[0], FilterExpression::Group { .. }));
    assert!(matches!(&members[1], FilterExpression::Predicate(_)));

    assert!(parse_filter_request("NOT LAYER = WALLS").is_some());
    assert!(parse_filter_request("XOR TYPE = LINE TYPE = ARC").is_some());
    assert_eq!(parse_filter_request("XOR TYPE = LINE"), None);
    assert_eq!(parse_filter_request("AND"), None);
    // Legacy `PROPERTY VALUE` has no operator and is not a logic expression.
    assert_eq!(parse_filter_request("LAYER WALLS"), None);
    let reason = invalid_filter_logic_argument_reason("FILTER OR");
    assert!(reason.contains("AND|OR|XOR|NOT"), "{reason}");

    let session = fresh_session();
    session
        .command("FILTER AND LAYER = WALLS TYPE = LINE")
        .expect("logic query runs");
    session
        .command("FILTER LAYER WALLS")
        .expect("legacy query runs");
    let error = session.command("FILTER OR").unwrap_err();
    assert!(error.contains("AND|OR|XOR|NOT"), "{error}");
}
