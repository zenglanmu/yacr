//! `SHADOWS`/`FOG` typed lines and the `display` host read.
//!
//! The session parses the line and dispatches the matching command; the panel's
//! read projects the open document's `tables/display.json` record through
//! `HostCore::display`. Contract level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;
use crate::session::commands::{parse_fog_argument, parse_shadows_argument};
use cad_app::DisplayCommand;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_display_grammar_is_narrow_and_case_insensitive() {
    assert_eq!(
        parse_shadows_argument(""),
        Some(DisplayCommand::ReportShadows)
    );
    assert_eq!(
        parse_shadows_argument("on"),
        Some(DisplayCommand::Shadows(true))
    );
    assert_eq!(
        parse_shadows_argument(" OFF "),
        Some(DisplayCommand::Shadows(false))
    );
    assert_eq!(
        parse_shadows_argument("sun 90 45"),
        Some(DisplayCommand::Sun {
            azimuth: 90.0,
            elevation: 45.0
        })
    );
    for refused in ["ON extra", "SUN", "SUN 90", "SUN a b", "BOGUS"] {
        assert_eq!(parse_shadows_argument(refused), None, "{refused:?}");
    }
    assert_eq!(parse_fog_argument(""), Some(DisplayCommand::ReportFog));
    assert_eq!(parse_fog_argument("ON"), Some(DisplayCommand::Fog(true)));
    assert_eq!(
        parse_fog_argument("RANGE 2 20"),
        Some(DisplayCommand::FogRange {
            start: 2.0,
            end: 20.0
        })
    );
    assert_eq!(
        parse_fog_argument("color 9 8 7"),
        Some(DisplayCommand::FogColor([9, 8, 7]))
    );
    assert_eq!(
        parse_fog_argument("COLOUR 1 2 3"),
        Some(DisplayCommand::FogColor([1, 2, 3]))
    );
    for refused in ["RANGE 1", "COLOR 1 2", "COLOR 999 0 0", "BOGUS"] {
        assert_eq!(parse_fog_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("FOG RANGE 2 20"),
        Some(("FOG", "RANGE 2 20"))
    );
    assert_eq!(typed_command_argument("FOG"), None);
    assert_eq!(
        typed_command_argument("SHADOWS ON"),
        Some(("SHADOWS", "ON"))
    );
    assert_eq!(command_name(CommandId::Shadows), Some("SHADOWS"));
    assert_eq!(command_name(CommandId::Fog), Some("FOG"));
}

#[test]
fn the_display_lines_write_the_document_and_a_bare_selector_only_reports() {
    let session = fresh_session();
    session.command("SHADOWS ON").expect("shadows on");
    session.command("SHADOWS SUN 90 30").expect("sun");
    session.command("FOG RANGE 2 20").expect("range");
    session.command("FOG COLOR 9 8 7").expect("colour");
    session.command("FOG ON").expect("fog on");
    {
        let c = session.core.controller.borrow();
        assert!(c.application.can_undo(&c.document_id));
    }
    let answer = session.display();
    assert!(answer.shadows.enabled);
    assert_eq!(answer.shadows.azimuth, 90.0);
    assert_eq!(answer.shadows.elevation, 30.0);
    assert!(answer.fog.enabled);
    assert_eq!((answer.fog.start, answer.fog.end), (2.0, 20.0));
    assert_eq!(answer.fog.color, [9, 8, 7]);
    // A bare selector reports; the write count does not move.
    let revision = {
        let c = session.core.controller.borrow();
        c.drawing().map(|db| db.revision())
    };
    session.command("SHADOWS").expect("report");
    session.command("FOG").expect("report");
    let after = {
        let c = session.core.controller.borrow();
        c.drawing().map(|db| db.revision())
    };
    assert_eq!(revision, after);
    // Undo returns the fog record one step at a time.
    session.command("UNDO").expect("undo fog on");
    assert!(!session.display().fog.enabled);
}

#[test]
fn the_display_host_read_projects_the_document_record() {
    let session = fresh_session();
    let default = session.display();
    assert!(!default.shadows.enabled);
    assert!(!default.fog.enabled);
    session.command("FOG ON").expect("fog on");
    assert!(session.display().fog.enabled);
}

#[test]
fn the_display_request_round_trips_through_the_envelope() {
    use crate::host_read::{HostReadAnswer, HostReadRequest};
    let request = HostReadRequest::Display;
    assert_eq!(request.kind(), crate::display::DISPLAY_KIND);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({ "kind": "display" })
    );
    let decoded: HostReadRequest =
        serde_json::from_value(serde_json::json!({ "kind": "display" })).expect("decode");
    assert_eq!(decoded, HostReadRequest::Display);
    let answer = HostReadAnswer::Display(crate::display::DisplayResponse::from_host(
        cad_domain::display::DisplaySettings::default(),
    ));
    assert!(matches!(answer, HostReadAnswer::Display(_)));
}

#[test]
fn a_carrier_without_a_display_lane_names_its_own_refusal() {
    assert_eq!(
        crate::host_read::no_seam_refusal(&crate::host_read::HostReadRequest::Display),
        crate::display::DISPLAY_UNSUPPORTED
    );
}
