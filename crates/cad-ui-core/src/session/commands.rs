//! The canonical command vocabulary and name normalization.

use cad_app::{CommandId, MeasurementToolKind};
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
    ("PLINE", CommandId::CreatePolyline),
    ("RECTANG", CommandId::CreateRectangle),
    ("ELLIPSE", CommandId::CreateEllipse),
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

/// Interaction-only selectors: canonical uppercase names that carry **no**
/// [`CommandId`] of their own and dispatch a typed
/// [`cad_app::interaction::InteractionRequest`] directly.
///
/// The four measurement selectors name the four real
/// [`MeasurementToolKind`] algorithms, so a measurement button starts *its own*
/// kind instead of every button collapsing onto `MEASURE`/Distance. `DIST`
/// (distance) and `AREA` are the real AutoCAD command names with exactly this
/// meaning. `MEASUREPOLYLINE` (polyline length) and `MEASUREANGLE` are explicit
/// project selectors: AutoCAD's `PLINE` now names the polyline **draw** command
/// (a [`COMMAND_TABLE`] entry, not a measurement selector) and `ANGLE` is not an
/// AutoCAD command at all, so neither measurement meaning shadows a real
/// command. `MEASURE` remains the default [`MeasurementToolKind::Distance`]
/// selector.
///
/// These names live in this table (never a hand-written UI list):
/// [`admissible_names`] folds them into the single vocabulary a frontend enables
/// from, so a new measurement selector is admitted without a second list.
pub const MEASUREMENT_SELECTORS: &[(&str, MeasurementToolKind)] = &[
    ("DIST", MeasurementToolKind::Distance),
    ("MEASUREPOLYLINE", MeasurementToolKind::PolylineLength),
    ("MEASUREANGLE", MeasurementToolKind::Angle),
    ("AREA", MeasurementToolKind::Area),
];

/// The zoom-in step factor for the `ZOOMIN` selector.
///
/// A zoom button supplies no cursor, so the session applies this fixed factor
/// about the **viewport centre** (see `CadSession::command`): a real zoom about
/// the viewport centre, never a fabricated no-op.
pub const ZOOM_IN_FACTOR: f64 = 1.25;

/// The zoom-out step factor for the `ZOOMOUT` selector (the reciprocal of
/// [`ZOOM_IN_FACTOR`], so the two steps are exact opposites).
pub const ZOOM_OUT_FACTOR: f64 = 1.0 / ZOOM_IN_FACTOR;

/// Fixed zoom-step selectors: canonical uppercase name -> factor about the
/// viewport centre. Distinct from the payload-bearing `ZOOM` selector, which
/// still requires a caller-supplied factor.
pub const ZOOM_STEP_SELECTORS: &[(&str, f64)] =
    &[("ZOOMIN", ZOOM_IN_FACTOR), ("ZOOMOUT", ZOOM_OUT_FACTOR)];

/// The canonical name in `table` matching the already-uppercased `key`, if any.
fn table_name<T>(table: &[(&'static str, T)], key: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(name, _)| *name)
}

/// The measurement kind an interaction-only selector names, if any.
///
/// The selector must already be canonical (as returned by
/// [`canonical_command_name`]); an alias never reaches here because aliases map
/// onto a canonical selector first.
pub fn measurement_selector_kind(name: &str) -> Option<MeasurementToolKind> {
    MEASUREMENT_SELECTORS
        .iter()
        .find(|(selector, _)| *selector == name)
        .map(|(_, kind)| *kind)
}

/// The fixed zoom factor a zoom-step selector names, if any.
pub fn zoom_step_factor(name: &str) -> Option<f64> {
    ZOOM_STEP_SELECTORS
        .iter()
        .find(|(selector, _)| *selector == name)
        .map(|(_, factor)| *factor)
}

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
/// `None` when the name maps to no admitted command.
///
/// Covers [`COMMAND_TABLE`], the interaction-only [`MEASUREMENT_SELECTORS`] /
/// [`ZOOM_STEP_SELECTORS`] and [`COMMAND_ALIASES`]. Names are matched
/// case-insensitively after trimming surrounding whitespace; internal whitespace
/// is significant (multi-word selectors keep one space).
pub fn canonical_command_name(text: &str) -> Option<&'static str> {
    let key = text.trim().to_ascii_uppercase();
    if let Some(name) = table_name(COMMAND_TABLE, &key) {
        return Some(name);
    }
    if let Some(name) = table_name(MEASUREMENT_SELECTORS, &key) {
        return Some(name);
    }
    if let Some(name) = table_name(ZOOM_STEP_SELECTORS, &key) {
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
/// unmapped name — including the interaction-only [`MEASUREMENT_SELECTORS`] /
/// [`ZOOM_STEP_SELECTORS`], which carry no `CommandId` — returns `None` so the
/// caller can dispatch or refuse it explicitly.
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
/// Canonical [`COMMAND_TABLE`] selectors come first, then the interaction-only
/// selectors ([`MEASUREMENT_SELECTORS`], [`ZOOM_STEP_SELECTORS`]), then every
/// alias that is not already canonical. Order is stable, so a frontend may sort
/// it for display without losing the canonical-first grouping.
pub fn admissible_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = COMMAND_TABLE.iter().map(|(name, _)| *name).collect();
    for (name, _) in MEASUREMENT_SELECTORS {
        if !names.contains(name) {
            names.push(name);
        }
    }
    for (name, _) in ZOOM_STEP_SELECTORS {
        if !names.contains(name) {
            names.push(name);
        }
    }
    for (alias, _) in COMMAND_ALIASES {
        if !names.contains(alias) {
            names.push(alias);
        }
    }
    names
}
