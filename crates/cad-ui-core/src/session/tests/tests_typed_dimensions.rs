//! `CadTool.params.lineLength` / `lineAngle` / `rectWidth` / `rectHeight` projection.
//!
//! The concept Measurements box renders the controls only when the active tool
//! projects the values, so this pins the wire contract: LINE projects both (the
//! `0` unset sentinel included), every other tool projects `null`, and the names
//! stay camelCase.

use super::super::*;
use super::test_support::*;

use cad_app::{ToolParameter, ToolValue};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn line_dimensions_are_projected_only_while_line_is_active() {
    let session = fresh_session();
    session.command("LINE").expect("LINE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("LINE is active");
    assert_eq!(tool.key, "draw.line");
    assert_eq!(tool.params.line_length, Some(0.0));
    assert_eq!(tool.params.line_angle, Some(0.0));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["lineLength"], serde_json::json!(0.0));
    assert_eq!(json["params"]["lineAngle"], serde_json::json!(0.0));

    session
        .set_tool_parameter(ToolParameter::LineLength, ToolValue::Number(12.5))
        .expect("a positive length is valid");
    session
        .set_tool_parameter(ToolParameter::LineAngle, ToolValue::Number(-30.0))
        .expect("any finite angle is valid");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("LINE is still active");
    assert_eq!(tool.params.line_length, Some(12.5));
    assert_eq!(tool.params.line_angle, Some(-30.0));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["lineLength"], serde_json::json!(12.5));
    assert_eq!(json["params"]["lineAngle"], serde_json::json!(-30.0));

    // Every other tool projects both fields as null (present, never omitted).
    for command in ["CIRCLE", "RECTANG", "PLINE"] {
        let other = fresh_session();
        other.command(command).expect("the tool is admitted");
        let tool = other
            .snapshot(None, None, 0)
            .tool
            .expect("a tool is active");
        assert_eq!(tool.params.line_length, None, "{command}");
        assert_eq!(tool.params.line_angle, None, "{command}");
        let json = serde_json::to_value(&tool).expect("CadTool serializes");
        assert_eq!(
            json["params"]["lineLength"],
            serde_json::Value::Null,
            "{command}"
        );
        assert_eq!(
            json["params"]["lineAngle"],
            serde_json::Value::Null,
            "{command}"
        );
    }
}

#[test]
fn a_negative_line_length_is_refused_and_a_zero_angle_is_accepted() {
    let session = fresh_session();
    session.command("LINE").expect("LINE is admitted");
    assert!(session
        .set_tool_parameter(ToolParameter::LineLength, ToolValue::Number(-1.0))
        .is_err());
    assert!(session
        .set_tool_parameter(ToolParameter::LineLength, ToolValue::Number(0.0))
        .is_ok());
    assert!(session
        .set_tool_parameter(ToolParameter::LineAngle, ToolValue::Number(0.0))
        .is_ok());
    assert!(session
        .set_tool_parameter(ToolParameter::LineAngle, ToolValue::Number(f64::NAN))
        .is_err());
}

#[test]
fn rectangle_extents_are_projected_only_while_rectang_is_active() {
    let session = fresh_session();
    session.command("RECTANG").expect("RECTANG is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("RECTANG is active");
    assert_eq!(tool.key, "draw.rectang");
    assert_eq!(tool.params.rect_width, Some(0.0));
    assert_eq!(tool.params.rect_height, Some(0.0));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["rectWidth"], serde_json::json!(0.0));
    assert_eq!(json["params"]["rectHeight"], serde_json::json!(0.0));

    session
        .set_tool_parameter(ToolParameter::RectWidth, ToolValue::Number(20.0))
        .expect("a positive width is valid");
    session
        .set_tool_parameter(ToolParameter::RectHeight, ToolValue::Number(5.0))
        .expect("a positive height is valid");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("RECTANG is still active");
    assert_eq!(tool.params.rect_width, Some(20.0));
    assert_eq!(tool.params.rect_height, Some(5.0));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["rectWidth"], serde_json::json!(20.0));
    assert_eq!(json["params"]["rectHeight"], serde_json::json!(5.0));

    // LINE projects both rectangle fields as null (present, never omitted).
    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.rect_width, None);
    assert_eq!(tool.params.rect_height, None);
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["rectWidth"], serde_json::Value::Null);
    assert_eq!(json["params"]["rectHeight"], serde_json::Value::Null);
}

#[test]
fn circle_radius_is_projected_only_while_circle_is_active() {
    let session = fresh_session();
    session.command("CIRCLE").expect("CIRCLE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("CIRCLE is active");
    assert_eq!(tool.key, "draw.circle");
    assert_eq!(tool.params.circle_radius, Some(0.0));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["circleRadius"], serde_json::json!(0.0));

    session
        .set_tool_parameter(ToolParameter::CircleRadius, ToolValue::Number(5.0))
        .expect("a positive radius is valid");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("CIRCLE is still active");
    assert_eq!(tool.params.circle_radius, Some(5.0));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["circleRadius"], serde_json::json!(5.0));

    // RECTANG projects the circle radius as null (present, never omitted).
    let rect = fresh_session();
    rect.command("RECTANG").expect("RECTANG is admitted");
    let tool = rect
        .snapshot(None, None, 0)
        .tool
        .expect("RECTANG is active");
    assert_eq!(tool.params.circle_radius, None);
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["circleRadius"], serde_json::Value::Null);
}

#[test]
fn rotate_angle_is_projected_only_while_rotate_is_active() {
    let session = fresh_session();
    session.command("SELECTALL").expect("SELECTALL is admitted");
    session.command("ROTATE").expect("ROTATE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("ROTATE is active");
    assert_eq!(tool.key, "draw.rotate");
    assert_eq!(tool.params.rotate_angle, Some(0.0));

    session
        .set_tool_parameter(ToolParameter::RotateAngle, ToolValue::Number(-30.0))
        .expect("any finite angle is valid");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("ROTATE is still active");
    assert_eq!(tool.params.rotate_angle, Some(-30.0));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["rotateAngle"], serde_json::json!(-30.0));

    // A drawing tool projects the rotate angle as null (present, never omitted).
    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.rotate_angle, None);
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["rotateAngle"], serde_json::Value::Null);
}

#[test]
fn move_distance_is_projected_only_while_move_is_active() {
    let session = fresh_session();
    session.command("SELECTALL").expect("SELECTALL is admitted");
    session.command("MOVE").expect("MOVE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("MOVE is active");
    assert_eq!(tool.key, "draw.move");
    assert_eq!(tool.params.move_distance, Some(0.0));

    session
        .set_tool_parameter(ToolParameter::MoveDistance, ToolValue::Number(12.5))
        .expect("a positive distance is valid");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("MOVE is still active");
    assert_eq!(tool.params.move_distance, Some(12.5));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["moveDistance"], serde_json::json!(12.5));

    // A drawing tool projects the move distance as null (present, never omitted).
    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.move_distance, None);
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["moveDistance"], serde_json::Value::Null);
}

#[test]
fn scale_factor_is_projected_only_while_scale_is_active() {
    let session = fresh_session();
    session.command("SELECTALL").expect("SELECTALL is admitted");
    session.command("SCALE").expect("SCALE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("SCALE is active");
    assert_eq!(tool.key, "draw.scale");
    assert_eq!(tool.params.scale_factor, Some(0.0));

    session
        .set_tool_parameter(ToolParameter::ScaleFactor, ToolValue::Number(2.5))
        .expect("a positive factor is valid");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("SCALE is still active");
    assert_eq!(tool.params.scale_factor, Some(2.5));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["scaleFactor"], serde_json::json!(2.5));

    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.scale_factor, None);
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["scaleFactor"], serde_json::Value::Null);
}

#[test]
fn offset_distance_is_projected_only_while_offset_is_active() {
    let session = fresh_session();
    session.command("SELECTALL").expect("SELECTALL is admitted");
    session.command("OFFSET").expect("OFFSET is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("OFFSET is active");
    assert_eq!(tool.key, "draw.offset");
    assert_eq!(tool.params.offset_distance, Some(0.0));

    session
        .set_tool_parameter(ToolParameter::OffsetDistance, ToolValue::Number(5.0))
        .expect("a positive distance is valid");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("OFFSET is still active");
    assert_eq!(tool.params.offset_distance, Some(5.0));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["offsetDistance"], serde_json::json!(5.0));

    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.offset_distance, None);
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["offsetDistance"], serde_json::Value::Null);
}

#[test]
fn extrude_distance_is_projected_only_while_extrude_is_active() {
    let session = fresh_session();
    session
        .command("EXTRUDE")
        .expect("EXTRUDE is admitted and starts its zero-point tool");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("EXTRUDE is active");
    assert_eq!(tool.key, "draw.extrude");
    assert_eq!(tool.params.extrude_distance, Some(0.0));

    session
        .set_tool_parameter(ToolParameter::ExtrudeDistance, ToolValue::Number(-3.5))
        .expect("a negative distance is valid");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("EXTRUDE is still active");
    assert_eq!(tool.params.extrude_distance, Some(-3.5));
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["extrudeDistance"], serde_json::json!(-3.5));

    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.extrude_distance, None);
    let json = serde_json::to_value(&tool).expect("CadTool serializes");
    assert_eq!(json["params"]["extrudeDistance"], serde_json::Value::Null);
}
