//! The Styles panel's status projection: the live `VSCURRENT` token.
//!
//! `visual_style` is session state (no document field). The snapshot reports the
//! session's real token so the tray can highlight it; the panel only dispatches
//! the existing `VSCURRENT <token>` selector. Contract level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_snapshot_reports_the_session_body_style_token() {
    let session = fresh_session();
    // The session default is the filled body display.
    assert_eq!(
        session.snapshot(None, None, 0).visual_style.as_deref(),
        Some("SHADED")
    );
    session
        .command("VSCURRENT WIREFRAME")
        .expect("VSCURRENT is admitted");
    assert_eq!(
        session.snapshot(None, None, 0).visual_style.as_deref(),
        Some("WIREFRAME")
    );
    session
        .command("VSCURRENT SHADED")
        .expect("VSCURRENT is admitted");
    assert_eq!(
        session.snapshot(None, None, 0).visual_style.as_deref(),
        Some("SHADED")
    );
}

#[test]
fn every_other_style_token_is_refused_by_name_and_keeps_the_current_style() {
    let session = fresh_session();
    for refused in ["REALISTIC", "CONCEPTUAL", "HIDDEN", "XRAY"] {
        let error: String = match session.command(&format!("VSCURRENT {refused}")) {
            Err(text) => text,
            Ok(()) => panic!("{refused} must be refused"),
        };
        assert!(
            error.contains("vscurrent.style_not_implemented"),
            "{refused}: {error}"
        );
        assert_eq!(
            session.snapshot(None, None, 0).visual_style.as_deref(),
            Some("SHADED"),
            "{refused} must not change the session token"
        );
    }
}
