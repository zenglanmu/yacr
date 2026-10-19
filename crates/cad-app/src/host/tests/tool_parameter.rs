//! General typed tool-parameter channel lifecycle (ADR 0016 decision A).
//!
//! Split out of `polygon.rs` so the narrow POLYGON migration and the general
//! channel each stay readable. The POLYGON-specific lifecycle (default 4, the
//! capture → confirm mapping) stays in `polygon.rs`; this file pins the **general**
//! channel: every parameter's documented default, a non-POLYGON parameter's
//! lifecycle (switch/cancel/edit keep, rebuild/new document reset, Viewer refuse),
//! and the per-parameter validation surface the session applies before storing.

use super::*;

use crate::{ToolParameter, ToolValue};

/// The ADR 0016 decision A lifecycle contract for the general channel: a
/// non-POLYGON parameter is kept across a tool switch, a cancel and a document
/// **edit**, refused out of range / wrong kind, reset by a session rebuild (new
/// blank document), and refused in Viewer mode.
#[test]
fn set_tool_parameter_lifecycle_and_validation_for_a_general_parameter() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();

    // Every parameter starts at its documented default.
    for parameter in ToolParameter::ALL {
        assert_eq!(
            controller.session.tool_parameter(parameter),
            parameter.default_value(),
            "{} starts at its documented default",
            parameter.key()
        );
    }

    // A non-POLYGON numeric parameter (FILLET radius) survives switch/cancel/edit.
    controller
        .session
        .set_tool_parameter(ToolParameter::FilletRadius, ToolValue::Number(12.5))
        .unwrap();
    assert_eq!(
        controller
            .session
            .tool_parameter(ToolParameter::FilletRadius),
        ToolValue::Number(12.5)
    );
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.cancel_tool().unwrap();
    assert_eq!(
        controller
            .session
            .tool_parameter(ToolParameter::FilletRadius),
        ToolValue::Number(12.5),
        "the parameter survives a tool switch and a cancel"
    );
    controller.create_line(p(0.0, 0.0), p(1.0, 0.0)).unwrap();
    assert_eq!(
        controller
            .session
            .tool_parameter(ToolParameter::FilletRadius),
        ToolValue::Number(12.5),
        "a document edit does not reset the parameter"
    );

    // A boolean parameter round-trips through the same channel.
    controller
        .session
        .set_tool_parameter(ToolParameter::AlignScale, ToolValue::Flag(true))
        .unwrap();
    assert_eq!(
        controller.session.tool_parameter(ToolParameter::AlignScale),
        ToolValue::Flag(true)
    );

    // Out of range is refused explicitly, never clamped, and never mutates.
    for bad in [-1.0, f64::NAN] {
        let error = controller
            .session
            .set_tool_parameter(ToolParameter::FilletRadius, ToolValue::Number(bad))
            .unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)), "radius={bad}");
        assert_eq!(
            controller
                .session
                .tool_parameter(ToolParameter::FilletRadius),
            ToolValue::Number(12.5),
            "a refused value must not mutate the parameter"
        );
    }

    // A wrong `ToolValue` kind is refused (a Flag for a numeric parameter).
    let error = controller
        .session
        .set_tool_parameter(ToolParameter::FilletRadius, ToolValue::Flag(true))
        .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)));

    // The fill angle accepts 0 < |angle| <= 360 (so the full-circle default 360 is
    // re-settable) and refuses 0 or anything beyond a single turn (no implicit
    // wrap).
    for good in [90.0, 360.0, -360.0] {
        controller
            .session
            .set_tool_parameter(ToolParameter::ArrayFillAngle, ToolValue::Number(good))
            .unwrap_or_else(|error| panic!("angle={good} refused: {error}"));
    }
    for bad in [0.0, 360.5, 720.0, -720.0] {
        let error = controller
            .session
            .set_tool_parameter(ToolParameter::ArrayFillAngle, ToolValue::Number(bad))
            .unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)), "angle={bad}");
    }
    controller
        .session
        .set_tool_parameter(ToolParameter::ArrayFillAngle, ToolValue::Number(90.0))
        .unwrap();
    assert_eq!(
        controller
            .session
            .tool_parameter(ToolParameter::ArrayFillAngle),
        ToolValue::Number(90.0)
    );

    // A session rebuild returns every parameter to its default.
    controller.reset_for_new_content();
    assert_eq!(
        controller
            .session
            .tool_parameter(ToolParameter::FilletRadius),
        ToolValue::Number(0.0),
        "a session rebuild resets the parameter to the default"
    );
    assert_eq!(
        controller.session.tool_parameter(ToolParameter::AlignScale),
        ToolValue::Flag(false)
    );
    controller
        .session
        .set_tool_parameter(ToolParameter::ArrayItems, ToolValue::Number(9.0))
        .unwrap();
    controller.new_blank_document().unwrap();
    assert_eq!(
        controller.session.tool_parameter(ToolParameter::ArrayItems),
        ToolValue::Number(1.0),
        "a new blank document resets the parameter to the default"
    );

    // Viewer mode refuses the parameter channel (Work-only create/edit commands).
    controller.set_mode(AppMode::Viewer).unwrap();
    let error = controller
        .session
        .set_tool_parameter(ToolParameter::FilletRadius, ToolValue::Number(3.0))
        .unwrap_err();
    assert!(
        matches!(error, CadError::PermissionDenied),
        "a Viewer-mode parameter set must be refused: {error:?}"
    );
    assert_eq!(
        controller
            .session
            .tool_parameter(ToolParameter::FilletRadius),
        ToolValue::Number(0.0)
    );
}
