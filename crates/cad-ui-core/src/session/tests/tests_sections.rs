//! `SECTIONPLANE` typed grammar and selector contracts.

use super::super::*;
use super::test_support::*;
use crate::session::commands::{
    invalid_section_argument_reason, parse_section_argument, SectionArgument,
};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_section_grammar_admits_only_the_shipped_options() {
    assert_eq!(parse_section_argument("ON"), Some(SectionArgument::On));
    assert_eq!(parse_section_argument("off"), Some(SectionArgument::Off));
    assert_eq!(
        parse_section_argument("TOGGLE"),
        Some(SectionArgument::Toggle)
    );
    assert_eq!(
        parse_section_argument("clear"),
        Some(SectionArgument::Clear)
    );
    assert_eq!(parse_section_argument("LIST"), Some(SectionArgument::List));
    match parse_section_argument("SET 1 2 3 0 0 2") {
        Some(SectionArgument::Set { origin, normal }) => {
            assert_eq!((origin.x, origin.y, origin.z), (1.0, 2.0, 3.0));
            assert_eq!((normal.x, normal.y, normal.z), (0.0, 0.0, 2.0));
        }
        other => panic!("expected SET, got {other:?}"),
    }
    for refused in [
        "",
        "SET",
        "SET 1 2 3 0 0",
        "SET 1 2 3 0 0 NaN",
        "ON NOW",
        "MAYBE",
    ] {
        assert_eq!(parse_section_argument(refused), None, "{refused}");
    }
    assert!(invalid_section_argument_reason("SECTIONPLANE BOGUS").contains("SECTIONPLANE BOGUS"));
}

#[test]
fn section_plane_is_a_real_router_selector() {
    assert_eq!(command_name(CommandId::SectionPlane), Some("SECTIONPLANE"));
    assert_eq!(
        normalize_command("SECTIONPLANE"),
        Some(CommandId::SectionPlane)
    );
    let session = fresh_session();
    for line in [
        "SECTIONPLANE LIST",
        "SECTIONPLANE SET 0 0 0 0 0 1",
        "SECTIONPLANE OFF",
        "SECTIONPLANE ON",
        "SECTIONPLANE CLEAR",
    ] {
        session
            .command(line)
            .unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    let error = session.command("SECTIONPLANE BOGUS").unwrap_err();
    assert!(error.contains("unsupported CAD command"), "{error}");
}

#[test]
fn the_status_projects_the_body_edges_session_token() {
    let session = fresh_session();
    assert_eq!(session.snapshot(None, None, 0).body_edges, Some(true));
    session.command("EDGES").expect("toggle body edges");
    assert_eq!(session.snapshot(None, None, 0).body_edges, Some(false));
    assert_eq!(
        command_name(CommandId::Extended(cad_app::ExtendedCommandId::Edges)),
        Some("EDGES")
    );
}

#[test]
fn edgecolor_grammar_and_status_projection() {
    use crate::session::commands::{parse_edge_color_argument, EdgeColorArgument};
    assert_eq!(
        parse_edge_color_argument("10 20 30"),
        Some(EdgeColorArgument::Set([10, 20, 30]))
    );
    assert_eq!(
        parse_edge_color_argument("DEFAULT"),
        Some(EdgeColorArgument::Default)
    );
    assert_eq!(
        parse_edge_color_argument(""),
        Some(EdgeColorArgument::Report)
    );
    assert_eq!(parse_edge_color_argument("10 20 300"), None);
    let session = fresh_session();
    assert_eq!(session.snapshot(None, None, 0).body_edge_color, None);
    session.command("EDGECOLOR 1 2 3").expect("set");
    assert_eq!(
        session.snapshot(None, None, 0).body_edge_color,
        Some([1, 2, 3])
    );
    session.command("EDGECOLOR DEFAULT").expect("reset");
    assert_eq!(session.snapshot(None, None, 0).body_edge_color, None);
}
