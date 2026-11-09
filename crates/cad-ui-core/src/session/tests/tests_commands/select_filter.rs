//! The 2026-11-07 selection/filter/find/visibility vocabulary and typed
//! argument grammar.
//!
//! Pins the canonical selectors, the `PROPERTY VALUE` / `needle => replacement`
//! grammar, the stable local refusals (unknown property, GETSEL's narrower set,
//! `-FIND` never replacing) and the Work-only split (`FIND` replace only). The
//! semantic match set is pinned in `cad-app`'s contract tests; this fixture has
//! no drawing, so a well-formed line stops at its `document not open` refusal,
//! which is exactly the proof that the line reached the application layer.

use super::super::super::commands::{
    invalid_audit_argument_reason, invalid_filter_argument_reason, invalid_find_argument_reason,
    invalid_getsel_argument_reason, invalid_ssx_argument_reason, parse_audit_argument,
    parse_find_argument, parse_property_filter, parse_ssx_argument,
};
use super::super::super::*;
use super::super::test_support::*;

#[test]
fn the_select_filter_batch_is_ten_canonical_selectors() {
    for (name, id) in [
        ("QSELECT", CommandId::QSelect),
        ("FILTER", CommandId::FilterSelect),
        ("GETSEL", CommandId::GetSelection),
        ("SSX", CommandId::SelectMatchingProperties),
        ("SELECTCOUNT", CommandId::SelectCount),
        ("-FIND", CommandId::FindReport),
        ("FIND", CommandId::FindReplace),
        ("HIDEOBJECTS", CommandId::HideObjects),
        ("ISOLATEOBJECTS", CommandId::IsolateObjects),
        ("UNISOLATEOBJECTS", CommandId::UnisolateObjects),
    ] {
        assert_eq!(canonical_command_name(name), Some(name), "{name}");
        assert_eq!(normalize_command(name), Some(id), "{name}");
        assert_eq!(command_name(id), Some(name), "{name}");
        assert!(
            admissible_names().contains(&name),
            "{name} is published to the frontend"
        );
    }
    // Only the replacing FIND writes the drawing.
    assert!(CommandId::FindReplace.requires_work_mode());
    for id in [
        CommandId::FindReport,
        CommandId::QSelect,
        CommandId::FilterSelect,
        CommandId::GetSelection,
        CommandId::SelectMatchingProperties,
        CommandId::SelectCount,
        CommandId::HideObjects,
        CommandId::IsolateObjects,
        CommandId::UnisolateObjects,
    ] {
        assert!(!id.requires_work_mode(), "{id:?} is session-state only");
    }
}

#[test]
fn filter_grammar_is_property_plus_trimmed_value() {
    assert_eq!(
        typed_command_argument("QSELECT TYPE LINE"),
        Some(("QSELECT", "TYPE LINE"))
    );
    assert_eq!(
        typed_command_argument("  filter\tlayer  WALLS "),
        Some(("FILTER", "layer  WALLS"))
    );
    assert_eq!(
        typed_command_argument("GETSEL TYPE CIRCLE"),
        Some(("GETSEL", "TYPE CIRCLE"))
    );
    // A bare selector keeps the name-only funnel (the application refuses it).
    assert_eq!(typed_command_argument("QSELECT"), None);
    // SSX and AUDIT are argument-taking selectors (bare forms stay name-only).
    assert_eq!(typed_command_argument("SSX 1"), Some(("SSX", "1")));
    assert_eq!(
        typed_command_argument("SSX BLOCK CHAIR"),
        Some(("SSX", "BLOCK CHAIR"))
    );
    assert_eq!(typed_command_argument("AUDIT YES"), Some(("AUDIT", "YES")));
    assert_eq!(typed_command_argument("AUDIT"), None);

    assert_eq!(
        parse_property_filter("TYPE AcDbLine"),
        Some(("TYPE".into(), "AcDbLine".into()))
    );
    assert_eq!(
        parse_property_filter("linetype my-dash"),
        Some(("LINETYPE".into(), "my-dash".into()))
    );
    for refused in ["", "TYPE", "WEIGHT 2", "TYPE   ", "COLOR\t"] {
        assert_eq!(parse_property_filter(refused), None, "{refused:?}");
    }
    let reason = invalid_filter_argument_reason("QSELECT WEIGHT 2", "QSELECT");
    assert!(reason.contains("TYPE|LAYER|COLOR|LINETYPE"), "{reason}");
    let reason = invalid_getsel_argument_reason("GETSEL COLOR aci:1");
    assert!(reason.contains("LAYER|TYPE"), "{reason}");
}

#[test]
fn find_grammar_splits_on_the_arrow_and_report_only_never_replaces() {
    assert_eq!(
        typed_command_argument("FIND hello"),
        Some(("FIND", "hello"))
    );
    assert_eq!(
        typed_command_argument("FIND hello => hi there"),
        Some(("FIND", "hello => hi there"))
    );
    assert_eq!(
        typed_command_argument("-FIND hello"),
        Some(("-FIND", "hello"))
    );
    assert_eq!(
        typed_command_argument("FIND"),
        None,
        "bare FIND is name-only"
    );

    assert_eq!(parse_find_argument(" hello "), Some(("hello".into(), None)));
    assert_eq!(
        parse_find_argument("hello => Hi There"),
        Some(("hello".into(), Some("Hi There".into())))
    );
    assert_eq!(
        parse_find_argument("hello =>"),
        Some(("hello".into(), Some(String::new())))
    );
    for refused in ["", "   ", "=> x"] {
        assert_eq!(parse_find_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_find_argument_reason("-FIND a => b", "-FIND");
    assert!(reason.contains("never"), "{reason}");
}

#[test]
fn well_formed_lines_reach_the_application_and_refusals_stay_local() {
    // Template/first-selection commands refuse the fresh session's empty
    // selection by name (proving the line reached the application). Checked
    // **before** any filter runs: a successful QSELECT replaces the selection
    // with a real demo-document match, which would give SSX a template.
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    for (line, expected) in [
        ("SSX", "ssx.selection_required"),
        ("SELECTCOUNT", "selectcount.selection_required"),
        ("HIDEOBJECTS", "hideobjects.selection_required"),
        ("ISOLATEOBJECTS", "isolateobjects.selection_required"),
    ] {
        let error = session.command(line).unwrap_err();
        assert!(error.contains(expected), "{line}: {error}");
    }

    // Well-formed, payload-complete lines reach the application: the demo
    // document answers a filter or a report, and neither is an error.
    for line in [
        "QSELECT TYPE LINE",
        "FILTER LAYER WALLS",
        "GETSEL TYPE CIRCLE",
        "FIND hello => hi",
        "-FIND hello",
        "UNISOLATEOBJECTS",
    ] {
        session
            .command(line)
            .unwrap_or_else(|error| panic!("{line}: {error}"));
    }

    // Bare selectors are refused by name, never defaulted.
    let error = session.command("QSELECT").unwrap_err();
    assert!(error.contains("QSELECT needs a SelectFilter"), "{error}");
    let error = session.command("FIND").unwrap_err();
    assert!(error.contains("FIND expects TextFind"), "{error}");

    // Locally invalid arguments never reach the host.
    let error = session.command("QSELECT WEIGHT 2").unwrap_err();
    assert!(
        error.contains("TYPE|LAYER|COLOR|LINETYPE"),
        "local grammar refusal: {error}"
    );
    let error = session.command("GETSEL COLOR aci:1").unwrap_err();
    assert!(error.contains("LAYER|TYPE"), "{error}");
    let error = session.command("-FIND a => b").unwrap_err();
    assert!(error.contains("never replaces"), "{error}");
}

#[test]
fn ssx_and_audit_typed_grammars() {
    use cad_app::{SsxCriterion, SsxCriterionKind, SsxRequest};

    assert_eq!(
        parse_ssx_argument("BLOCK CHAIR"),
        Some(SsxRequest::Set(SsxCriterion::Block("CHAIR".into())))
    );
    assert_eq!(
        parse_ssx_argument("layer My Wall"),
        Some(SsxRequest::Set(SsxCriterion::Layer("My Wall".into())))
    );
    assert_eq!(
        parse_ssx_argument("COLOR 1"),
        Some(SsxRequest::Set(SsxCriterion::Color("1".into())))
    );
    assert_eq!(
        parse_ssx_argument("ENTITY CIRCLE"),
        Some(SsxRequest::Set(SsxCriterion::Entity("CIRCLE".into())))
    );
    assert_eq!(
        parse_ssx_argument("FLAG"),
        Some(SsxRequest::Set(SsxCriterion::Flag))
    );
    assert_eq!(
        parse_ssx_argument("LTYPE DASHED"),
        Some(SsxRequest::Set(SsxCriterion::LType("DASHED".into())))
    );
    assert_eq!(
        parse_ssx_argument("STYLE Note"),
        Some(SsxRequest::Set(SsxCriterion::Style("Note".into())))
    );
    assert_eq!(
        parse_ssx_argument("THICKNESS 2.5"),
        Some(SsxRequest::Set(SsxCriterion::Thickness(2.5)))
    );
    assert_eq!(
        parse_ssx_argument("VECTOR 0 0 1"),
        Some(SsxRequest::Set(SsxCriterion::Vector(cad_domain::Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        })))
    );
    assert_eq!(parse_ssx_argument("PICK"), Some(SsxRequest::Pick));
    assert_eq!(
        parse_ssx_argument("CLEAR LAYER"),
        Some(SsxRequest::Clear(SsxCriterionKind::Layer))
    );

    for refused in [
        "",
        "BLOCK",
        "FLAG x",
        "THICKNESS abc",
        "THICKNESS NaN",
        "VECTOR 1 2",
        "VECTOR 1 2 3 4",
        "CLEAR",
        "CLEAR NOPE",
        "NOPE 1",
    ] {
        assert_eq!(parse_ssx_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_ssx_argument_reason("SSX NOPE 1");
    assert!(reason.contains("BLOCK|COLOR|ENTITY|FLAG|LAYER"), "{reason}");

    for (line, fix, log) in [
        ("YES", true, false),
        ("y", true, false),
        ("NO", false, false),
        ("n", false, false),
        ("LOG", false, true),
    ] {
        let request = parse_audit_argument(line).expect(line);
        assert_eq!((request.fix, request.auditctl_log), (fix, log), "{line}");
    }
    for refused in ["", "MAYBE", "YES LOG"] {
        assert_eq!(parse_audit_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_audit_argument_reason("AUDIT MAYBE");
    assert!(reason.contains("YES"), "{reason}");
}
