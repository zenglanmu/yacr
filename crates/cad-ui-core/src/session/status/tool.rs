//! The frozen `cad_status.tool` projection (active capture → wire shape).
//!
//! Split out of `status.rs` to keep that module inside the code-size limit; the
//! function is re-exported by the parent under its original name, so every call
//! site and the wire shape itself are unchanged.

use super::*;

/// Project the real interactive tool state onto the frozen wire shape.
///
/// Idle/selecting/panning map to `None` (no capture is active). A drawing or
/// measurement capture carries its `cad-app` machine key, the **real** captured
/// point count and `cad-app`'s own readiness; nothing is inferred from the
/// pointer stream or fabricated for a tool the state machine does not hold.
///
/// `session` is the authoritative session whose typed parameter store supplies the
/// projected values (POLYGON's `sides`, FILLET's `filletRadius`, CHAMFER's
/// `chamferDistanceA`/`B`, ARRAYRECT's four grid parameters, ARRAYPOLAR's item
/// count / fill angle / rotate flag, TEXT/MTEXT's `textHeight`/`textRotation` and
/// MTEXT's `mtextAttachment`); each is present **only** while its own tool is
/// active, so a control's enablement is derived from `tool.key` and never from a
/// stale local value. No value is fabricated for an inactive tool.
pub(in crate::session) fn cad_tool(
    snapshot: &ToolSnapshot,
    session: &cad_app::SessionState,
) -> Option<CadTool> {
    use cad_app::{DrawToolKind, ToolParameter};

    let number = |parameter: ToolParameter, active: bool| {
        active
            .then(|| session.tool_parameter(parameter).number())
            .flatten()
    };
    let flag = |parameter: ToolParameter, active: bool| {
        active
            .then(|| session.tool_parameter(parameter).flag())
            .flatten()
    };
    match snapshot {
        ToolSnapshot::Drawing(preview) => Some(CadTool {
            key: format!("draw.{}", preview.kind.key()),
            points: preview.points.len(),
            ready: preview.can_confirm(),
            // The real arity model: `true` only for the open-ended kinds (PLINE,
            // SPLINE) while active, so the UI never reports a fixed "remaining"
            // for a capture that still accepts points. Never inferred from the
            // captured count.
            open_ended: preview.is_open_ended(),
            // The real text prompt phase (ADR 0019 decision A.2), read from
            // `cad-app`'s own capture state — never inferred from `key`/`points`.
            awaiting_text: preview.awaiting_text,
            // The TEXTEDIT prefill, straight from the capture state: `None` for
            // every other kind and before the pick resolves.
            text_prefill: preview.text_prefill.clone(),
            params: CadToolParams {
                // The side count is present **only** for a POLYGON capture; every
                // other draw tool reports `null`, and the session already clears
                // the field for a non-POLYGON kind.
                sides: if matches!(
                    preview.kind,
                    DrawToolKind::Polygon | DrawToolKind::PolygonCircumscribed
                ) {
                    preview.polygon_sides
                } else {
                    None
                },
                fillet_radius: number(
                    ToolParameter::FilletRadius,
                    preview.kind == DrawToolKind::Fillet,
                ),
                chamfer_distance_a: number(
                    ToolParameter::ChamferDistanceA,
                    preview.kind == DrawToolKind::Chamfer,
                ),
                chamfer_distance_b: number(
                    ToolParameter::ChamferDistanceB,
                    preview.kind == DrawToolKind::Chamfer,
                ),
                // The four ARRAYRECT parameters are present **only** while an
                // ARRAYRECT capture is active.
                array_rows: number(
                    ToolParameter::ArrayRows,
                    preview.kind == DrawToolKind::ArrayRect,
                ),
                array_columns: number(
                    ToolParameter::ArrayColumns,
                    preview.kind == DrawToolKind::ArrayRect,
                ),
                array_row_spacing: number(
                    ToolParameter::ArrayRowSpacing,
                    preview.kind == DrawToolKind::ArrayRect,
                ),
                array_column_spacing: number(
                    ToolParameter::ArrayColumnSpacing,
                    preview.kind == DrawToolKind::ArrayRect,
                ),
                // The three ARRAYPOLAR parameters are present **only** while an
                // ARRAYPOLAR capture is active.
                array_items: number(
                    ToolParameter::ArrayItems,
                    preview.kind == DrawToolKind::ArrayPolar,
                ),
                array_fill_angle: number(
                    ToolParameter::ArrayFillAngle,
                    preview.kind == DrawToolKind::ArrayPolar,
                ),
                array_rotate_items: flag(
                    ToolParameter::ArrayRotateItems,
                    preview.kind == DrawToolKind::ArrayPolar,
                ),
                // The ALIGN scale flag is present **only** while an ALIGN capture
                // is active (ADR 0016 decision F); it defaults to false.
                align_scale: flag(
                    ToolParameter::AlignScale,
                    preview.kind == DrawToolKind::Align,
                ),
                // The pair count selects ALIGN's capture shape; it is projected
                // beside the scale flag while an ALIGN capture is active.
                align_pairs: number(
                    ToolParameter::AlignPairs,
                    preview.kind == DrawToolKind::Align,
                ),
                // BREAK's `F` override selects its capture shape.
                break_first_point: flag(
                    ToolParameter::BreakFirstPoint,
                    preview.kind == DrawToolKind::Break,
                ),
                // DIVIDE's block option opens the terminal block-name step.
                divide_use_block: flag(
                    ToolParameter::DivideUseBlock,
                    preview.kind == DrawToolKind::Divide,
                ),
                // DIVIDE's block-align choice rotates the inserted blocks to the
                // curve tangent; it is read only by the block path but stays
                // visible (inert) in the point form.
                divide_align_block: flag(
                    ToolParameter::DivideAlignBlock,
                    preview.kind == DrawToolKind::Divide,
                ),
                // The six HATCH parameters are present **only** while a HATCH
                // capture is active (ADR 0018 decision D, P6b): the tool captures
                // no points, so these are its whole definition. HATCHPOINT reads
                // the **same** ten parameters, so its capture projects them too.
                hatch_pattern_type: number(
                    ToolParameter::HatchPatternType,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                hatch_pattern: number(
                    ToolParameter::HatchPattern,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                hatch_spacing: number(
                    ToolParameter::HatchSpacing,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                hatch_angle: number(
                    ToolParameter::HatchAngle,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                hatch_double: flag(
                    ToolParameter::HatchDouble,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                hatch_style: number(
                    ToolParameter::HatchStyle,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                // The four gradient parameters ride the same HATCH capture (they
                // are only read when the pattern type is the gradient code `3`).
                hatch_gradient_kind: number(
                    ToolParameter::HatchGradientKind,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                hatch_gradient_angle: number(
                    ToolParameter::HatchGradientAngle,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                hatch_gradient_color_a: number(
                    ToolParameter::HatchGradientColorA,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                hatch_gradient_color_b: number(
                    ToolParameter::HatchGradientColorB,
                    matches!(preview.kind, DrawToolKind::Hatch | DrawToolKind::HatchPoint),
                ),
                // The text height is present **only** while a text-carrying
                // creation capture is active (ADR 0019 decision F.1). TEXT and
                // MTEXT both create their text with it, so both project it; the
                // two edit kinds (TEXTEDIT, MLEADER) have no height channel.
                text_height: number(
                    ToolParameter::TextHeight,
                    matches!(preview.kind, DrawToolKind::Text | DrawToolKind::MText),
                ),
                // The creation rotation (degrees) rides the same two kinds: TEXT
                // reads it directly at creation, MTEXT converts it into its
                // payload. No other kind stores a rotation parameter.
                text_rotation: number(
                    ToolParameter::TextRotation,
                    matches!(preview.kind, DrawToolKind::Text | DrawToolKind::MText),
                ),
                // MTEXT is the only tool with an attachment channel; TEXT's
                // alignment is the style default and never parameterised.
                mtext_attachment: number(
                    ToolParameter::MTextAttachment,
                    preview.kind == DrawToolKind::MText,
                ),
                // The single-curve edit pair (2026-11-01) projects its own rows
                // only while the matching zero-point capture is active: DIVIDE owns
                // the segment count, LENGTHEN the mode and the amount.
                divide_segments: number(
                    ToolParameter::DivideSegments,
                    preview.kind == DrawToolKind::Divide,
                ),
                lengthen_mode: number(
                    ToolParameter::LengthenMode,
                    preview.kind == DrawToolKind::Lengthen,
                ),
                lengthen_value: number(
                    ToolParameter::LengthenValue,
                    preview.kind == DrawToolKind::Lengthen,
                ),
                // SCALETEXT owns its factor only while the matching zero-point
                // capture is active.
                scale_text_factor: number(
                    ToolParameter::ScaleTextFactor,
                    preview.kind == DrawToolKind::ScaleText,
                ),
                // DIMROTATED owns its angle only while the matching three-pick
                // capture is active (the channel carries degrees).
                dim_rotation: number(
                    ToolParameter::DimRotation,
                    preview.kind == DrawToolKind::DimRotated,
                ),
                // PLINE reads one uniform width when it builds the polyline, so
                // only that capture projects it (null for every other draw kind).
                polyline_width: number(
                    ToolParameter::PolylineWidth,
                    preview.kind == DrawToolKind::Polyline,
                ),
                // STRETCH's crossing-window mode is projected only while a STRETCH
                // capture is active, so the capture-mode chooser can never show a
                // value from another tool's lifetime.
                stretch_window: number(
                    ToolParameter::StretchWindow,
                    preview.kind == DrawToolKind::Stretch,
                ),
                line_length: number(
                    ToolParameter::LineLength,
                    preview.kind == DrawToolKind::Line,
                ),
                line_angle: number(ToolParameter::LineAngle, preview.kind == DrawToolKind::Line),
                rect_width: number(
                    ToolParameter::RectWidth,
                    preview.kind == DrawToolKind::Rectang,
                ),
                rect_height: number(
                    ToolParameter::RectHeight,
                    preview.kind == DrawToolKind::Rectang,
                ),
                circle_radius: number(
                    ToolParameter::CircleRadius,
                    preview.kind == DrawToolKind::Circle,
                ),
                rotate_angle: number(
                    ToolParameter::RotateAngle,
                    preview.kind == DrawToolKind::Rotate,
                ),
                move_distance: number(
                    ToolParameter::MoveDistance,
                    preview.kind == DrawToolKind::Move,
                ),
                scale_factor: number(
                    ToolParameter::ScaleFactor,
                    preview.kind == DrawToolKind::Scale,
                ),
                offset_distance: number(
                    ToolParameter::OffsetDistance,
                    preview.kind == DrawToolKind::Offset,
                ),
                press_pull_distance: number(
                    ToolParameter::PressPullDistance,
                    preview.kind == DrawToolKind::PressPull,
                ),
                extrude_distance: number(
                    ToolParameter::ExtrudeDistance,
                    preview.kind == DrawToolKind::Extrude,
                ),
            },
        }),
        ToolSnapshot::Measuring(preview) => Some(CadTool {
            key: format!("measure.{}", preview.kind.key()),
            points: preview.points.len(),
            ready: preview.can_confirm(),
            // A measurement capture is never open-ended; report the honest
            // `false` rather than a fabricated `true`.
            open_ended: false,
            // A measurement never awaits text.
            awaiting_text: false,
            // …and never carries a text prefill.
            text_prefill: None,
            params: CadToolParams {
                sides: None,
                fillet_radius: None,
                chamfer_distance_a: None,
                chamfer_distance_b: None,
                array_rows: None,
                array_columns: None,
                array_row_spacing: None,
                array_column_spacing: None,
                array_items: None,
                array_fill_angle: None,
                array_rotate_items: None,
                align_scale: None,
                align_pairs: None,
                break_first_point: None,
                divide_use_block: None,
                divide_align_block: None,
                hatch_pattern_type: None,
                hatch_pattern: None,
                hatch_spacing: None,
                hatch_angle: None,
                hatch_double: None,
                hatch_style: None,
                hatch_gradient_kind: None,
                hatch_gradient_angle: None,
                hatch_gradient_color_a: None,
                hatch_gradient_color_b: None,
                text_height: None,
                text_rotation: None,
                mtext_attachment: None,
                divide_segments: None,
                lengthen_mode: None,
                lengthen_value: None,
                scale_text_factor: None,
                dim_rotation: None,
                polyline_width: None,
                stretch_window: None,
                line_length: None,
                line_angle: None,
                rect_width: None,
                rect_height: None,
                circle_radius: None,
                rotate_angle: None,
                move_distance: None,
                scale_factor: None,
                offset_distance: None,
                press_pull_distance: None,
                extrude_distance: None,
            },
        }),
        ToolSnapshot::Idle | ToolSnapshot::Selecting | ToolSnapshot::Panning => None,
    }
}
