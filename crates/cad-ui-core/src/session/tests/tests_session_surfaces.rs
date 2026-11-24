//! The two chrome-facing session tokens on `cad_status` (2026-11-24):
//! `textScreen` (`TEXTSCR`/`GRAPHSCR`) and `traySettings` (`TRAYSETTINGS`).
//!
//! These are **projections**, not a second source: the chrome's text screen and
//! status-bar tray read the session's own `drafting_extras` flags through the
//! snapshot. The wire test pins the camelCase keys the frontend parses.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn text_screen_and_tray_flags_follow_their_commands_on_the_snapshot() {
    let session = fresh_session();
    let initial = session.snapshot(None, None, 0);
    assert!(!initial.text_screen, "TEXTSCR starts closed");
    assert!(
        initial.tray_settings,
        "TRAYSETTINGS starts ON (AutoCAD default)"
    );

    session.command("TEXTSCR").expect("TEXTSCR is admitted");
    let opened = session.snapshot(None, None, 0);
    assert!(opened.text_screen, "TEXTSCR opens the host flag");

    session.command("GRAPHSCR").expect("GRAPHSCR is admitted");
    let closed = session.snapshot(None, None, 0);
    assert!(!closed.text_screen, "GRAPHSCR closes it");

    session
        .command("TRAYSETTINGS")
        .expect("TRAYSETTINGS is admitted");
    let hidden = session.snapshot(None, None, 0);
    assert!(!hidden.tray_settings, "TRAYSETTINGS flips the tray flag");
    session
        .command("TRAYSETTINGS")
        .expect("TRAYSETTINGS toggles back");
    assert!(session.snapshot(None, None, 0).tray_settings);

    // No drawing write: the document facts on the snapshot are unchanged.
    assert_eq!(closed.entity_count, initial.entity_count);
    assert_eq!(hidden.entity_count, initial.entity_count);
    assert!(
        hidden.command_log.len() > initial.command_log.len(),
        "the commands are admitted and echoed, but they write no entity"
    );
}

#[test]
fn the_wire_carries_camel_case_text_screen_and_tray_settings_keys() {
    let session = fresh_session();
    session.command("TEXTSCR").expect("TEXTSCR is admitted");
    let wire = serde_json::to_value(session.snapshot(None, None, 0)).expect("serializes");
    assert_eq!(wire["textScreen"], serde_json::json!(true));
    assert_eq!(wire["traySettings"], serde_json::json!(true));
    session
        .command("TRAYSETTINGS")
        .expect("TRAYSETTINGS is admitted");
    let wire = serde_json::to_value(session.snapshot(None, None, 0)).expect("serializes");
    assert_eq!(
        wire["traySettings"],
        serde_json::json!(false),
        "the key is always present"
    );
}
