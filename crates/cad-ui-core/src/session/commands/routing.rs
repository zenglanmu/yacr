//! The selector -> interactive-tool routing table.
//!
//! Split out of `commands.rs` (which is at the file-size ceiling) with the
//! function body unchanged. `commands` re-exports it, so the
//! `commands::interaction_request_for` call site in `session` is unchanged.

use cad_app::CommandId;

/// The typed interaction request a selector starts, or `None` when the command
/// runs as an ordinary command (it needs a payload, or is a session-only toggle).
///
/// This is the **single** routing table between the command vocabulary and the
/// interactive tool captures: an `Some` value is submitted as
/// [`cad_app::interaction::InteractionRequest::BeginDraw`]/`BeginMeasurement`
/// and the tool owns the pointer from then on, while `None` falls through to
/// `dispatch_command` and the application refuses a payload-requiring command
/// explicitly instead of fabricating one.
///
/// Split out of `session::mod` (which stays inside the code-size limit) with its
/// body unchanged. The clipboard cluster is the one place where a kind pair
/// (`CopyBase`/`CutBase`) exists only because the typed request carries no
/// `CommandId`; the session records the command so the host can map the confirmed
/// payload by it.
pub(crate) fn interaction_request_for(
    id: CommandId,
) -> Option<cad_app::interaction::InteractionRequest> {
    use cad_app::interaction::InteractionRequest;
    use cad_app::DrawToolKind;
    use cad_app::MeasurementToolKind;

    match id {
        CommandId::CreateLine => Some(InteractionRequest::BeginDraw(DrawToolKind::Line)),
        // XLINE/RAY start their two-point construction-line capture (base, then a
        // through point); the commit stores one finite clipped segment.
        CommandId::CreateXLine => Some(InteractionRequest::BeginDraw(DrawToolKind::XLine)),
        CommandId::CreateRay => Some(InteractionRequest::BeginDraw(DrawToolKind::Ray)),
        CommandId::CreateCircle => Some(InteractionRequest::BeginDraw(DrawToolKind::Circle)),
        // The CIRCLE construction options start their own captures: two diameter
        // endpoints (2P), three points (3P) and a centre plus a diameter point (D).
        CommandId::CreateCircle2P => Some(InteractionRequest::BeginDraw(DrawToolKind::Circle2P)),
        CommandId::CreateCircle3P => Some(InteractionRequest::BeginDraw(DrawToolKind::Circle3P)),
        CommandId::CreateCircleDiameter => {
            Some(InteractionRequest::BeginDraw(DrawToolKind::CircleDiameter))
        }
        CommandId::CreateArc => Some(InteractionRequest::BeginDraw(DrawToolKind::Arc)),
        CommandId::CreatePolyline => Some(InteractionRequest::BeginDraw(DrawToolKind::Polyline)),
        // WIPEOUT starts the open-ended polygon capture (first point, then every
        // further boundary point, Enter finishes); the command stores one mask.
        CommandId::Wipeout => Some(InteractionRequest::BeginDraw(DrawToolKind::Wipeout)),
        CommandId::CreateRectangle => Some(InteractionRequest::BeginDraw(DrawToolKind::Rectang)),
        CommandId::CreateEllipse => Some(InteractionRequest::BeginDraw(DrawToolKind::Ellipse)),
        CommandId::CreatePolygon => Some(InteractionRequest::BeginDraw(DrawToolKind::Polygon)),
        // POLYGON CIRCUMSCRIBED starts its own two-point capture (centre, then a
        // point on the tangent circle); the command derives the tangent ring.
        CommandId::CreatePolygonCircumscribed => Some(InteractionRequest::BeginDraw(
            DrawToolKind::PolygonCircumscribed,
        )),
        CommandId::CreatePoint => Some(InteractionRequest::BeginDraw(DrawToolKind::Point)),
        CommandId::CreateSpline => Some(InteractionRequest::BeginDraw(DrawToolKind::Spline)),
        // INSERT starts the point-then-name capture: one insertion point, then the
        // block name through the same terminal `cad_set_tool_text` channel TEXT
        // uses. The name resolves against the drawing's block metadata.
        CommandId::InsertBlock => Some(InteractionRequest::BeginDraw(DrawToolKind::InsertBlock)),
        // BLOCK takes the current selection as its members, one base point, then
        // the new block name through the same terminal `cad_set_tool_text`
        // channel. Work-only.
        CommandId::DefineBlock => Some(InteractionRequest::BeginDraw(DrawToolKind::BlockDefine)),
        // TEXT starts the text tool: its point complement is one insertion
        // point, then the terminal text arrives through the dedicated
        // `cad_set_tool_text` channel (ADR 0019 decision A.1) — never through
        // this command line, which carries a command name only.
        CommandId::CreateText => Some(InteractionRequest::BeginDraw(DrawToolKind::Text)),
        // MTEXT starts the two-corner text tool: the first corner is the
        // insertion point, the opposite corner supplies the wrap width, and
        // the terminal text arrives through the same `cad_set_tool_text`
        // channel TEXT uses (one text step, no second capture path).
        CommandId::CreateMText => Some(InteractionRequest::BeginDraw(DrawToolKind::MText)),
        // MLEADER starts the leader tool: two points (arrow, landing) then the
        // same terminal text channel TEXT uses (ADR 0019 decision F.2).
        CommandId::MLeader => Some(InteractionRequest::BeginDraw(DrawToolKind::MLeader)),
        // TEXTEDIT starts the edit tool: an entity pick of the text to edit,
        // then the same terminal text channel (ADR 0020 amendment A).
        CommandId::TextEdit => Some(InteractionRequest::BeginDraw(DrawToolKind::TextEdit)),
        CommandId::DimLinear => Some(InteractionRequest::BeginDraw(DrawToolKind::DimLinear)),
        CommandId::DimAligned => Some(InteractionRequest::BeginDraw(DrawToolKind::DimAligned)),
        CommandId::DimRadius => Some(InteractionRequest::BeginDraw(DrawToolKind::DimRadius)),
        CommandId::DimDiameter => Some(InteractionRequest::BeginDraw(DrawToolKind::DimDiameter)),
        CommandId::DimCenter => Some(InteractionRequest::BeginDraw(DrawToolKind::DimCenter)),
        CommandId::DimAngular => Some(InteractionRequest::BeginDraw(DrawToolKind::DimAngular)),
        CommandId::DimOrdinate => Some(InteractionRequest::BeginDraw(DrawToolKind::DimOrdinate)),
        CommandId::DimBaseline => Some(InteractionRequest::BeginDraw(DrawToolKind::DimBaseline)),
        CommandId::DimContinue => Some(InteractionRequest::BeginDraw(DrawToolKind::DimContinue)),
        CommandId::MoveEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Move)),
        CommandId::CopyEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Copy)),
        CommandId::RotateEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Rotate)),
        CommandId::ScaleEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Scale)),
        CommandId::MirrorEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Mirror)),
        CommandId::OffsetEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Offset)),
        // The clipboard cluster (`COPYCLIP`/`CUTCLIP` are instant commands, not
        // tools):
        //
        // * `COPYBASE` / `CUTBASE` capture a base point over the current
        //   selection. They share one capture shape but two kinds, because the
        //   typed `BeginDraw` request carries the kind and **no** `CommandId`
        //   while the two commands differ in authorization (`COPYBASE` is
        //   Viewer-legal, `CUTBASE` is work-only) and in the confirmed payload
        //   (capture only vs capture **and** delete). The session records the
        //   command, and the host maps the confirmed intent to the payload by
        //   it.
        // * `PASTECLIP` captures one insertion point (no selection: the
        //   clipboard is the input) and is work-only through its command.
        // * `PASTEORIG` takes no point at all, so it falls through to
        //   `dispatch_command` and inserts at the original coordinates.
        CommandId::CopyToClipboardBase => {
            Some(InteractionRequest::BeginDraw(DrawToolKind::CopyBase))
        }
        CommandId::CutToClipboardBase => Some(InteractionRequest::BeginDraw(DrawToolKind::CutBase)),
        CommandId::PasteFromClipboard => {
            Some(InteractionRequest::BeginDraw(DrawToolKind::PasteClip))
        }
        // PASTEBLOCK captures one insertion point over the clipboard and turns it
        // into a new auto-named block definition; work-only through its command.
        CommandId::PasteBlock => Some(InteractionRequest::BeginDraw(DrawToolKind::PasteBlock)),
        CommandId::TrimEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Trim)),
        CommandId::ExtendEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Extend)),
        // BREAK: the first click is the entity pick **and** the first break
        // point, then the second break point.
        CommandId::Break => Some(InteractionRequest::BeginDraw(DrawToolKind::Break)),
        // DIVIDE/LENGTHEN are the zero-point pre-selection shape: the selected
        // curve is the whole input, so the selector only starts the tool.
        CommandId::Divide => Some(InteractionRequest::BeginDraw(DrawToolKind::Divide)),
        CommandId::Lengthen => Some(InteractionRequest::BeginDraw(DrawToolKind::Lengthen)),
        // SCALETEXT is the same zero-point pre-selection shape: the selected text
        // entities are the whole input, so the selector only starts the tool.
        CommandId::ScaleText => Some(InteractionRequest::BeginDraw(DrawToolKind::ScaleText)),
        CommandId::ChamferLines => Some(InteractionRequest::BeginDraw(DrawToolKind::Chamfer)),
        CommandId::FilletLines => Some(InteractionRequest::BeginDraw(DrawToolKind::Fillet)),
        CommandId::ArrayRectangular => Some(InteractionRequest::BeginDraw(DrawToolKind::ArrayRect)),
        CommandId::ArrayPolar => Some(InteractionRequest::BeginDraw(DrawToolKind::ArrayPolar)),
        // HATCH has the pre-selection shape: its boundary is the current
        // selection, so the selector only starts the tool (no payload).
        CommandId::Hatch => Some(InteractionRequest::BeginDraw(DrawToolKind::Hatch)),
        // HATCHPOINT captures one internal point; the command discovers the
        // boundary from it (the selection is not its input).
        CommandId::HatchPoint => Some(InteractionRequest::BeginDraw(DrawToolKind::HatchPoint)),
        CommandId::AlignEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Align)),
        CommandId::StretchEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Stretch)),
        CommandId::Measure => Some(InteractionRequest::BeginMeasurement(
            MeasurementToolKind::Distance,
        )),
        // The bare `PAN` selector is the modal-pan affordance, exactly like
        // the other interactive tools: `Application::pan` needs a `Points`
        // world delta a selector cannot supply, so routing it through
        // `BeginPan` is what makes the button work instead of erroring.
        CommandId::Pan => Some(InteractionRequest::BeginPan),
        CommandId::ConfirmMeasurement => Some(InteractionRequest::Confirm),
        CommandId::CancelLoading => Some(InteractionRequest::Cancel),
        // MTEXT has its own arm above (the two-corner tool). Every remaining
        // selector either needs a payload or is handled by a typed channel, so
        // it falls through to `dispatch_command` with `CommandPayload::None`;
        // the application then refuses it explicitly instead of fabricating a
        // payload. Such names are nonetheless real, advertised selectors: the
        // payload path (CLI/host IPC) uses them.
        _ => None,
    }
}
