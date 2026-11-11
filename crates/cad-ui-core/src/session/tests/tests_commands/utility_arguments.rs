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
fn purge_and_rename_arguments_are_admitted_then_refused_by_the_command_layer() {
    let session = fresh_session();
    // The default drawing has no block table delete API; the host refusal is the
    // named boundary, never an empty success.
    let error = session.command("PURGE BLOCK").unwrap_err();
    assert!(error.contains("purge.block.unsupported"), "{error}");
    let error = session.command("RENAME LAYER NOPE X").unwrap_err();
    assert!(error.contains("layer.unknown"), "{error}");
    let error = session.command("RENAME LAYER A").unwrap_err();
    assert!(error.contains("invalid RENAME argument"), "{error}");
    let error = session.command("RENAME WIDGET A B").unwrap_err();
    assert!(error.contains("invalid RENAME argument"), "{error}");
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
