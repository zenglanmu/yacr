//! The typed utility/named-object lines (`PURGE`, `RENAME`, `SETVAR`, `MREDO`,
//! `MULTIPLE`) at the router boundary.
//!
//! The `cad-app` tests pin the command layer; these pin the command-line grammar
//! and, most importantly, that `SETVAR` writes reuse the existing variable
//! handlers instead of introducing a second write path.

use super::super::super::*;
use super::super::test_support::*;

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
fn setvar_reads_the_closed_set_and_refuses_unknown_names() {
    let session = fresh_session();
    for line in [
        "SETVAR PDMODE",
        "SETVAR PDSIZE",
        "SETVAR SNAPUNIT",
        "SETVAR LTSCALE",
        "SETVAR GRID",
        "SETVAR SNAP",
        "SETVAR ORTHO",
        "SETVAR TEXTSIZE",
    ] {
        session
            .command(line)
            .unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    let echoed: Vec<String> = session
        .snapshot(None, None, 0)
        .command_log
        .iter()
        .filter_map(|entry| entry.command.clone())
        .collect();
    assert_eq!(
        echoed
            .iter()
            .filter(|name| name.as_str() == "SETVAR")
            .count(),
        8,
        "each typed line echoes the canonical selector: {echoed:?}"
    );
    let error = session.command("SETVAR NOT_A_VAR").unwrap_err();
    assert!(error.contains("setvar.unknown_variable"), "{error}");
}

#[test]
fn setvar_writes_route_through_the_existing_variable_handlers() {
    let session = fresh_session();
    let before = drawing(&session).revision();

    session.command("SETVAR PDMODE 35").expect("$PDMODE write");
    assert_eq!(drawing(&session).point_display_mode(), 35);
    assert!(drawing(&session).revision() > before);

    session.command("SETVAR PDSIZE -20").expect("$PDSIZE write");
    assert_eq!(drawing(&session).point_display_size(), -20.0);

    session
        .command("SETVAR LTSCALE 2.5")
        .expect("$LTSCALE write");
    assert_eq!(drawing(&session).linetype_scale(), 2.5);

    // Session-only variables write the same session state their own selector
    // writes; nothing here is a second path.
    session
        .command("SETVAR SNAPUNIT 5")
        .expect("$SNAPUNIT write");
    assert_eq!(session.snapshot(None, None, 0).snap_spacing, 5.0);
    session.command("SETVAR GRID OFF").expect("GRID OFF");
    assert!(!session.snapshot(None, None, 0).grid_enabled);
    session.command("SETVAR SNAP ON").expect("SNAP ON");
    assert!(session.snapshot(None, None, 0).snap_grid_enabled);
    session.command("SETVAR ORTHO ON").expect("ORTHO ON");
    assert!(session.snapshot(None, None, 0).ortho_enabled);

    // EXPLMODE (also `EXPMODE`) is the session switch the EXPLODE path reads;
    // the write never touches the drawing.
    assert!(session.core.controller.borrow().session.explode_mode());
    session
        .command("SETVAR EXPLMODE 0")
        .expect("EXPLMODE write");
    assert!(!session.core.controller.borrow().session.explode_mode());
    session
        .command("SETVAR EXPMODE ON")
        .expect("EXPMODE alias write");
    assert!(session.core.controller.borrow().session.explode_mode());

    // TEXTSIZE is the session tool-parameter channel, not a drawing header.
    session
        .command("SETVAR TEXTSIZE 7.5")
        .expect("TEXTSIZE write");
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .tool_parameter(ToolParameter::TextHeight),
        ToolValue::Number(7.5)
    );
}

#[test]
fn setvar_refuses_a_bad_value_or_variable_by_name() {
    let session = fresh_session();
    let before = drawing(&session).revision();
    let error = session.command("SETVAR PDMODE 3.5").unwrap_err();
    assert!(error.contains("invalid SETVAR PDMODE value"), "{error}");
    let error = session.command("SETVAR MREDO 2").unwrap_err();
    assert!(error.contains("setvar.unknown_variable"), "{error}");
    assert_eq!(drawing(&session).revision(), before);
}

#[test]
fn purge_and_rename_arguments_are_admitted_then_handled_by_the_command_layer() {
    let session = fresh_session();
    // `PURGE BLOCK` is a real table purge now. The fresh drawing has no block row,
    // so the command reports the named no-op (`purge.tables_none`) instead of
    // inventing work, and the revision does not move.
    let before = drawing(&session).revision();
    session
        .command("PURGE BLOCK")
        .expect("PURGE BLOCK is admitted and finds nothing to remove");
    assert_eq!(drawing(&session).revision(), before);
    let error = session.command("RENAME LAYER NOPE X").unwrap_err();
    assert!(error.contains("layer.unknown"), "{error}");
    let error = session.command("RENAME LAYER A").unwrap_err();
    assert!(error.contains("invalid RENAME argument"), "{error}");
    let error = session.command("RENAME WIDGET A B").unwrap_err();
    assert!(error.contains("invalid RENAME argument"), "{error}");
}

#[test]
fn dash_purge_is_the_purge_handler_and_removes_an_unused_layer() {
    let session = fresh_session();
    session
        .command("-LAYER NEW UNUSED")
        .expect("the typed NEW creates a layer");
    assert!(drawing(&session).layers().any(|row| row.name == "UNUSED"));

    // `-PURGE` canonicalises onto `PURGE`; it is the same handler, not a second
    // implementation. The unused layer row is removed exactly as bare `PURGE`
    // would remove it.
    session
        .command("-PURGE")
        .expect("-PURGE is the command-line spelling of PURGE");
    let names: Vec<String> = drawing(&session)
        .layers()
        .map(|row| row.name.clone())
        .collect();
    assert!(
        !names.contains(&"UNUSED".to_string()),
        "the unused layer survived -PURGE: {names:?}"
    );
    assert!(
        names.contains(&"0".to_string()),
        "layer 0 is never a purge candidate: {names:?}"
    );
}

#[test]
fn mredo_and_multiple_validate_their_typed_arguments() {
    let session = fresh_session();
    let error = session.command("MREDO 0").unwrap_err();
    assert!(error.contains("invalid MREDO argument"), "{error}");
    let error = session.command("MREDO nope").unwrap_err();
    assert!(error.contains("invalid MREDO argument"), "{error}");

    let error = session.command("MULTIPLE NOPE").unwrap_err();
    assert!(error.contains("multiple.unknown_command"), "{error}");
    session.command("MULTIPLE LINE").expect("LINE is admitted");
}

#[test]
fn sysvarmonitor_typed_forms_add_remove_and_list_one_or_all() {
    let session = fresh_session();
    session
        .command("SYSVARMONITOR ADD SNAPUNIT")
        .expect("ADD is admitted");
    let shown = session.snapshot(None, None, 0);
    assert_eq!(
        shown
            .command_log
            .iter()
            .filter(|entry| entry.command.as_deref() == Some("SYSVARMONITOR"))
            .count(),
        1,
        "the typed line echoes its canonical selector"
    );
    session
        .command("SYSVARMONITOR LIST SNAPUNIT")
        .expect("LIST <name> is admitted");
    session
        .command("SYSVARMONITOR LIST")
        .expect("bare LIST is admitted");
    session
        .command("SYSVARMONITOR REMOVE SNAPUNIT")
        .expect("REMOVE is admitted");

    let error = session.command("SYSVARMONITOR ADD").unwrap_err();
    assert!(error.contains("invalid SYSVARMONITOR"), "{error}");
    let error = session.command("SYSVARMONITOR ADD NOT_A_VAR").unwrap_err();
    assert!(error.contains("sysvarmonitor.unknown_variable"), "{error}");
}

#[test]
fn julian_typed_date_routes_to_the_report_and_refuses_impossible_dates() {
    let session = fresh_session();
    session
        .command("JULIAN 2000-01-01T12:00:00")
        .expect("a real date is admitted");
    session
        .command("JULIAN 2026/11/24")
        .expect("the slash spelling is admitted");
    let echoed = session
        .snapshot(None, None, 0)
        .command_log
        .iter()
        .filter(|entry| entry.command.as_deref() == Some("JULIAN"))
        .count();
    assert_eq!(echoed, 2, "each typed line echoes the canonical selector");

    for line in [
        "JULIAN 2026-02-30",
        "JULIAN 2026-13-01",
        "JULIAN 2026-01-01T24:00",
        "JULIAN not-a-date",
    ] {
        let error = session.command(line).unwrap_err();
        assert!(
            error.contains("julian.invalid_date"),
            "{line}: expected a named refusal, got {error}"
        );
    }
}

#[test]
fn dwgprops_typed_lines_write_summary_metadata_and_name_unknown_fields() {
    let session = fresh_session();
    let before = drawing(&session).revision();
    session
        .command("DWGPROPS TITLE \"Pump station\"")
        .expect("TITLE write");
    session
        .command("DWGPROPS AUTHOR A. Drafter")
        .expect("unquoted spaces join the one value");
    session
        .command("DWGPROPS CUSTOM \"Project\" \"North\"")
        .expect("CUSTOM write");
    let after = drawing(&session);
    assert!(after.revision() > before);
    assert_eq!(
        after.document_metadata().title.as_deref(),
        Some("Pump station")
    );
    assert_eq!(
        after.document_metadata().author.as_deref(),
        Some("A. Drafter")
    );
    assert_eq!(
        after
            .document_metadata()
            .custom
            .get("Project")
            .map(String::as_str),
        Some("North")
    );
    // A quoted empty token clears; a bare field does not silently clear.
    session.command("DWGPROPS TITLE \"\"").expect("clear");
    assert!(drawing(&session).document_metadata().title.is_none());
    let error = session.command("DWGPROPS TITLE").unwrap_err();
    assert!(error.contains("dwgprops.invalid_argument"), "{error}");
    let error = session.command("DWGPROPS PRICE 3").unwrap_err();
    assert!(error.contains("dwgprops.unknown_field"), "{error}");
}

#[test]
fn units_typed_settings_lines_write_length_angle_and_direction() {
    let session = fresh_session();
    session
        .command("UNITS LENGTH architectural")
        .expect("length format write");
    session
        .command("UNITS ANGLE dms 3")
        .expect("angle format write");
    session
        .command("UNITS ANGLE DIRECTION north CW")
        .expect("direction write");
    {
        let controller = session.core.controller.borrow();
        let display = *controller.drawing().expect("a drawing").units_display();
        assert_eq!(display.length_format, cad_db::LengthFormat::Architectural);
        assert_eq!(
            display.angle_format,
            cad_db::AngleFormat::DegreesMinutesSeconds
        );
        assert_eq!(display.angle_precision, 3);
        assert_eq!(display.angle_base_degrees, 90.0);
        assert!(display.angle_clockwise);
    }
    // `other` needs the one angle it cannot derive.
    let error = session.command("UNITS ANGLE DIRECTION other").unwrap_err();
    assert!(error.contains("units.invalid_argument"), "{error}");
    let error = session.command("UNITS ANGLE dms 9").unwrap_err();
    assert!(error.contains("units.invalid_argument"), "{error}");
    let error = session.command("UNITS LENGTH bogus").unwrap_err();
    assert!(error.contains("units.invalid_argument"), "{error}");
    let error = session
        .command("UNITS ANGLE DIRECTION north 45 90")
        .unwrap_err();
    assert!(error.contains("units.invalid_argument"), "{error}");
}

#[test]
fn units_typed_lines_write_the_document_unit_and_refuse_bad_arguments() {
    let session = fresh_session();
    session.command("UNITS mm 2").expect("UNITS write");
    {
        let controller = session.core.controller.borrow();
        let units = &controller.application.workspace.documents[&controller.document_id].units;
        assert_eq!(units.display, cad_domain::Unit::Millimeter);
        assert_eq!(units.decimal_places, 2);
    }
    session.command("-UNITS m 4").expect("-UNITS write");
    {
        let controller = session.core.controller.borrow();
        let units = &controller.application.workspace.documents[&controller.document_id].units;
        assert_eq!(units.display, cad_domain::Unit::Meter);
        assert_eq!(units.decimal_places, 4);
    }
    let error = session.command("UNITS mm 9").unwrap_err();
    assert!(error.contains("units.invalid_argument"), "{error}");
    let error = session.command("UNITS parsec 2").unwrap_err();
    assert!(error.contains("units.invalid_argument"), "{error}");
}
