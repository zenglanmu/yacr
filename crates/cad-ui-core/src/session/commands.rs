//! The canonical command vocabulary and name normalization.

use cad_app::CommandId;
/// The canonical command vocabulary: every [`CommandId`] variant with the stable
/// uppercase selector a frontend uses to name it.
///
/// This is the single mapping table; it is deliberately exhaustive over the
/// `cad-app` command set so a host never has to special-case a command. A
/// selector whose variant needs a payload still dispatches through the same
/// funnel and is refused explicitly by the application/host layer.
pub const COMMAND_TABLE: &[(&str, CommandId)] = &[
    ("OPEN", CommandId::OpenDrawing),
    ("NEW", CommandId::NewDrawing),
    ("PLOT", CommandId::PlotDrawing),
    ("SAVE", CommandId::SaveDrawingAs),
    ("CANCEL", CommandId::CancelLoading),
    ("PAN", CommandId::Pan),
    ("ZOOM", CommandId::Zoom),
    ("FIT", CommandId::FitDrawing),
    ("RESETVIEW", CommandId::ResetView),
    ("TOGGLELAYER", CommandId::ToggleLayer),
    ("SETLAYERS", CommandId::SetLayerVisibilities),
    ("RESTORELAYERS", CommandId::RestoreLayers),
    ("SPACE", CommandId::SwitchSpace),
    ("SELECT", CommandId::Select),
    ("SELECTALL", CommandId::SelectAll),
    ("SELECTINBOX", CommandId::SelectInBox),
    ("MEASURE", CommandId::Measure),
    ("CONFIRM", CommandId::ConfirmMeasurement),
    ("CANCELMEASURE", CommandId::CancelMeasurement),
    ("UNDO", CommandId::Undo),
    ("REDO", CommandId::Redo),
    ("RESOURCES", CommandId::Resources),
    ("DIAGNOSTICS", CommandId::Diagnostics),
    ("SWITCHBACKEND", CommandId::SwitchBackend),
    ("SWITCH2D3D", CommandId::Switch2d3d),
    ("STANDARDVIEW", CommandId::StandardView),
    ("ORBIT", CommandId::Orbit),
    ("PROJECTION", CommandId::SwitchProjection),
    ("MODE", CommandId::SetMode),
    ("LINE", CommandId::CreateLine),
    ("CIRCLE", CommandId::CreateCircle),
    ("ARC", CommandId::CreateArc),
    ("MOVE", CommandId::MoveEntities),
    ("TRIM", CommandId::TrimEntity),
    ("EXTEND", CommandId::ExtendEntity),
    ("ERASE", CommandId::EraseEntities),
    ("COPY", CommandId::CopyEntities),
    ("ROTATE", CommandId::RotateEntities),
    ("SCALE", CommandId::ScaleEntities),
    ("MIRROR", CommandId::MirrorEntities),
    ("COPYCLIP", CommandId::CopyToClipboard),
    ("CUTCLIP", CommandId::CutToClipboard),
    ("PASTECLIP", CommandId::PasteFromClipboard),
    ("OFFSET", CommandId::OffsetEntity),
    ("SETACTIVELAYER", CommandId::SetActiveLayer),
    ("HOVERPREVIEW", CommandId::ToggleHoverPreview),
    ("OSNAP", CommandId::ToggleSnap),
];

/// Full-word command synonyms, always accepted, that map onto a canonical
/// [`COMMAND_TABLE`] selector.
///
/// `SNAP` is deliberately absent: AutoCAD's `SNAP` is **grid snap** (F9), which
/// this project does not implement and which is a different feature from the
/// object snap (`OSNAP`) selector. Aliasing it onto `ToggleSnap` would silently
/// turn a request for grid snap into an object-snap toggle.
pub const COMMAND_ALIASES: &[(&str, &str)] = &[
    ("EXPORT", "PLOT"),
    ("PRINT", "PLOT"),
    ("SAVEAS", "SAVE"),
    ("QSAVE", "SAVE"),
    ("ZOOM EXTENTS", "FIT"),
    ("ENTER", "CONFIRM"),
];

/// The canonical, stable uppercase selector a command name resolves to, or
/// `None` when the name maps to no [`CommandId`].
///
/// Names are matched case-insensitively after trimming surrounding whitespace;
/// internal whitespace is significant (multi-word selectors keep one space).
pub fn canonical_command_name(text: &str) -> Option<&'static str> {
    let key = text.trim().to_ascii_uppercase();
    if let Some((name, _)) = COMMAND_TABLE.iter().find(|(name, _)| *name == key) {
        return Some(name);
    }
    COMMAND_ALIASES
        .iter()
        .find(|(alias, _)| *alias == key)
        .and_then(|(_, canonical)| canonical_command_name(canonical))
}

/// The `CommandId` a command name names, if any.
///
/// Covers the full [`COMMAND_TABLE`] vocabulary (plus [`COMMAND_ALIASES`]); an
/// unmapped name returns `None` so the caller can refuse it explicitly.
pub fn normalize_command(text: &str) -> Option<CommandId> {
    let name = canonical_command_name(text)?;
    COMMAND_TABLE
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, id)| *id)
}

/// The stable rejection reason for a name that maps to no command.
pub fn unsupported_command_reason(text: &str) -> String {
    format!("unsupported CAD command: {}", text.trim())
}

/// Every name the host admits, for a frontend that must disable what cannot run
/// instead of keeping a hand-written mirror that drifts.
///
/// Canonical selectors come first, then every alias that is not already
/// canonical. Order is stable, so a frontend may sort it for display without
/// losing the canonical-first grouping.
pub fn admissible_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = COMMAND_TABLE.iter().map(|(name, _)| *name).collect();
    for (alias, _) in COMMAND_ALIASES {
        if !names.contains(alias) {
            names.push(alias);
        }
    }
    names
}
