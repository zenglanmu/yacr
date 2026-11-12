//! `CadTool.params.polylineWidth` projection for the PLINE uniform width.
//!
//! The React control renders only when the active tool projects the value, so
//! this pins the wire contract: PLINE projects its stored width (default `0`,
//! the hairline draw), every other tool projects `null`, and an idle or Viewer
//! session has no active tool at all. The null is serialized (never omitted).

use super::super::*;
use super::test_support::*;

use cad_app::{ToolParameter, ToolValue};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn polyline_width_is_projected_only_while_pline_is_active() {
    // PLINE projects its stored uniform width, starting at the hairline default.
    let session = fresh_session();
    session.command("PLINE").expect("PLINE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("PLINE is active");
    assert_eq!(tool.key, "draw.polyline");
    assert_eq!(
        tool.params.polyline_width,
        Some(0.0),
        "the hairline default is a real projected value"
    );
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(
        json["params"]["polylineWidth"],
        serde_json::json!(0.0),
        "the wire name is camelCase `polylineWidth`"
    );

    // A typed width is projected verbatim (the channel validated it).
    session
        .set_tool_parameter(ToolParameter::PolylineWidth, ToolValue::Number(1.5))
        .expect("1.5 is a valid uniform width");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("PLINE is still active");
    assert_eq!(tool.params.polyline_width, Some(1.5));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["polylineWidth"], serde_json::json!(1.5));

    // Another tool has the field at `null` — present in the JSON, not skipped.
    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.key, "draw.line");
    assert_eq!(tool.params.polyline_width, None);
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["polylineWidth"], serde_json::Value::Null);

    // Viewer mode idles the capture, and an idle session has no tool block, so
    // there is no width to project at all.
    let viewer = fresh_session();
    viewer.command("PLINE").expect("PLINE is admitted");
    viewer
        .core
        .controller
        .borrow_mut()
        .set_mode(cad_app::AppMode::Viewer)
        .expect("switch to Viewer");
    assert!(
        viewer.snapshot(None, None, 0).tool.is_none(),
        "a Viewer session carries no active tool"
    );
    let idle = fresh_session();
    assert!(idle.snapshot(None, None, 0).tool.is_none());
}
