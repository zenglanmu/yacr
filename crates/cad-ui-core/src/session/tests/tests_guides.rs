//! `DIST GUIDE` / `PROTRACTOR GUIDE` capture modes and the `GUIDE` management
//! grammar. Contract level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;
use crate::session::commands::{parse_dist_guide_argument, parse_guide_argument, GuideArgument};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn dist_guide_starts_the_distance_capture_that_will_write_a_guide() {
    assert!(parse_dist_guide_argument("guide"));
    assert!(!parse_dist_guide_argument(""));
    let session = fresh_session();
    session
        .command("DIST GUIDE")
        .expect("DIST GUIDE is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("a tool");
    assert_eq!(tool.key, "measure.distance");

    let text = match session.command("DIST BOGUS") {
        Err(text) => text,
        Ok(()) => panic!("DIST BOGUS must be refused"),
    };
    assert!(text.contains("unsupported CAD command"), "{text}");
}

#[test]
fn protractor_guide_starts_the_protractor_capture() {
    let session = fresh_session();
    session
        .command("PROTRACTOR GUIDE")
        .expect("PROTRACTOR GUIDE is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("a tool");
    assert_eq!(tool.key, "measure.protractor");
}

#[test]
fn guide_list_and_clear_are_the_only_management_forms() {
    assert_eq!(parse_guide_argument("list"), Some(GuideArgument::List));
    assert_eq!(parse_guide_argument("CLEAR"), Some(GuideArgument::Clear));
    assert_eq!(
        parse_guide_argument("DELETE 2"),
        Some(GuideArgument::Delete(2))
    );
    match parse_guide_argument("POINT 1 2 3") {
        Some(GuideArgument::Point(point)) => {
            assert_eq!((point.x, point.y, point.z), (1.0, 2.0, 3.0));
        }
        other => panic!("expected a point, got {other:?}"),
    }
    assert_eq!(parse_guide_argument("DELETE"), None);
    assert_eq!(parse_guide_argument("DELETE 0"), None);
    assert_eq!(parse_guide_argument("POINT 1 2"), None);
    assert_eq!(parse_guide_argument("POINT 1 NaN 3"), None);

    let session = fresh_session();
    for line in [
        "GUIDE DELETE",
        "GUIDE DELETE 0",
        "GUIDE CLEAR NOW",
        "GUIDE POINT 1 2",
    ] {
        let text = match session.command(line) {
            Err(text) => text,
            Ok(()) => panic!("{line} must be refused"),
        };
        assert!(text.contains("unsupported CAD command"), "{line}: {text}");
    }
    // A bare GUIDE is the list form and reaches the read-only command layer.
    session.command("GUIDE").expect("bare GUIDE lists");
    assert_eq!(normalize_command("GUIDE"), Some(CommandId::Guide));
}
