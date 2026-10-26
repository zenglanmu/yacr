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
    // LAYISO hides every layer the selection is not on (plus the current layer),
    // as session visibility overrides only — no payload, no database write. The
    // restore half is the existing `RESTORELAYERS` path: `LAYUNISO`/`LAYERP` are
    // aliases of it below, so there is exactly one restore.
    ("LAYISO", CommandId::IsolateLayers),
    // LAYOFF is its sibling: hide exactly the selection's layers (the current
    // layer is never turned off), again as session visibility overrides only.
    ("LAYOFF", CommandId::LayOff),
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
    ("POLYGON", CommandId::CreatePolygon),
    ("POINT", CommandId::CreatePoint),
    ("SPLINE", CommandId::CreateSpline),
    ("TEXT", CommandId::CreateText),
    // MTEXT is the multiline create command **and** a real interactive tool
    // (2026-10-26): the bare selector starts the two-corner capture — first
    // corner (insertion point), opposite corner (wrap width), then the terminal
    // text step. The payload path (`CommandPayload::MText`) stays available for
    // CLI/host IPC. No `MT` abbreviation is admitted: this table only lists
    // abbreviations whose behaviour it really implements end to end.
    ("MTEXT", CommandId::CreateMText),
    ("MLEADER", CommandId::MLeader),
    ("TEXTEDIT", CommandId::TextEdit),
    ("DIMLINEAR", CommandId::DimLinear),
    ("DIMALIGNED", CommandId::DimAligned),
    ("DIMRADIUS", CommandId::DimRadius),
    ("DIMDIAMETER", CommandId::DimDiameter),
    ("DIMCENTER", CommandId::DimCenter),
    ("DIMANGULAR", CommandId::DimAngular),
    ("DIMORDINATE", CommandId::DimOrdinate),
    ("DIMBASELINE", CommandId::DimBaseline),
    ("DIMCONTINUE", CommandId::DimContinue),
    ("HATCH", CommandId::Hatch),
    ("MOVE", CommandId::MoveEntities),
    ("TRIM", CommandId::TrimEntity),
    ("EXTEND", CommandId::ExtendEntity),
    ("CHAMFER", CommandId::ChamferLines),
    ("FILLET", CommandId::FilletLines),
    ("ARRAYRECT", CommandId::ArrayRectangular),
    ("ARRAYPOLAR", CommandId::ArrayPolar),
    ("ALIGN", CommandId::AlignEntities),
    ("STRETCH", CommandId::StretchEntities),
    ("ERASE", CommandId::EraseEntities),
    // JOIN merges the **selection** into one entity (collinear LINEs / concentric
    // ARCs in this build); the bare selector is its whole input, so no payload and
    // no IPC. The application refuses a shorter selection by name at apply time.
    ("JOIN", CommandId::Join),
    ("EXPLODE", CommandId::ExplodeEntities),
    ("COPY", CommandId::CopyEntities),
    ("ROTATE", CommandId::RotateEntities),
    ("SCALE", CommandId::ScaleEntities),
    ("MIRROR", CommandId::MirrorEntities),
    ("COPYCLIP", CommandId::CopyToClipboard),
    ("CUTCLIP", CommandId::CutToClipboard),
    ("PASTECLIP", CommandId::PasteFromClipboard),
    ("OFFSET", CommandId::OffsetEntity),
    ("SETACTIVELAYER", CommandId::SetActiveLayer),
    // Layer **table** writes (create/rename/delete). Each needs a payload — the
    // new name (create), `{ layer, name }` (rename) or `{ layer }` (delete) — so
    // the bare selector normalizes and is then refused explicitly by the
    // application layer; the panel/IPC path supplies the typed payload. The
    // layer selector inside the payload is a **name**, resolved against the
    // drawing's own table (exact match first, then a unique case-insensitive one).
    ("CREATELAYER", CommandId::CreateLayer),
    ("RENAMELAYER", CommandId::RenameLayer),
    ("DELETELAYER", CommandId::DeleteLayer),
    // The layer **attribute** write (`off`/`frozen`/`locked`/`plottable`/`color`/
    // `linetype`/`lineweight`/`transparency`) is also payload-requiring:
    // `{ layer, key, value }`. The bare selector normalizes and is refused
    // explicitly by the application layer; the panel/IPC path
    // (`cad_set_layer_property`) supplies the typed payload.
    ("SETLAYERPROPERTY", CommandId::SetLayerProperty),
    // A payload-requiring edit: the selector normalizes, and the application
    // refuses a bare invocation explicitly (the panel/IPC path supplies the
    // `EntityProperty { key, value }` payload).
    ("SETENTITYPROPERTY", CommandId::SetEntityProperty),
    // MATCHPROP is the **selection-based** property painter: the bare selector is
    // the whole input (`CommandPayload::None`), because the current selection's
    // first entity is the source and the rest are destinations. No IPC of its own.
    ("MATCHPROP", CommandId::MatchProp),
    // GRIPSTRETCH is the **payload path of a pointer gesture**: `GripStretch
    // { id, to }` carries the resolved handle id plus the world point the drag
    // resolved to. That payload comes from the host's own pointer routing on the
    // release of an armed grip drag (`cad_host`), never from this table: this
    // layer only maps the selector, and the typed command line carries no handle,
    // so the bare selector normalizes and is then refused explicitly by the
    // application layer — it must never be routed to `BeginDraw` or turned into a
    // fabricated drag.
    ("GRIPSTRETCH", CommandId::GripStretch),
    ("HOVERPREVIEW", CommandId::ToggleHoverPreview),
    // The display grid switch (F7). Grid **snap** is the separate `SNAP`
    // selector below; the two are independent session switches and never alias
    // each other.
    ("GRID", CommandId::ToggleGrid),
    // Grid snap (F9): quantise a captured point to the session `SNAPUNIT`
    // spacing. Independent of object snap (`OSNAP`) and of the display grid
    // (`GRID`); no alias maps onto any of the three.
    ("SNAP", CommandId::ToggleSnapGrid),
    ("OSNAP", CommandId::ToggleSnap),
    // The typed spacing sysvar: `SNAPUNIT <value>`. The bare name carries no
    // value, so the application refuses it explicitly (no fabricated default).
    ("SNAPUNIT", CommandId::SetSnapSpacing),
    // The orthographic drafting constraint (F8). Session/display state only;
    // applied after object snap and grid snap so it still decides the final axis.
    ("ORTHO", CommandId::ToggleOrtho),
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
    // ID is AutoCAD's own point-coordinate command (`Specify point`). It is an
    // interaction-only selector like the four above: the one pick is resolved by
    // the host (object snap included) and the answer is a point report, not a
    // measurement, so it carries no `CommandId` and never enters the measurement
    // engine.
    ("ID", MeasurementToolKind::Id),
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

/// The selectors that accept a **typed argument** on the command line.
///
/// An argument is admitted by its base selector and then validated against that
/// selector's own grammar; it is **not** a new [`CommandId`] and there is no
/// second IPC for it. Every other selector stays name-only, so `LINE 2` is the
/// ordinary unsupported-name refusal.
///
/// * [`ARGUMENT_SELECTOR`] (`ZOOM`): a positive factor or `ALL`/`EXTENTS`
///   ([`parse_zoom_argument`]);
/// * [`SNAPUNIT_SELECTOR`] (`SNAPUNIT`): a positive decimal spacing
///   ([`parse_snap_unit_argument`]).
pub const ARGUMENT_SELECTORS: &[&str] = &[ARGUMENT_SELECTOR, SNAPUNIT_SELECTOR];

/// AutoCAD's `ZOOM n` / `ZOOM ALL`: the first argument-taking selector.
///
/// A positive factor reuses the `ZOOMIN`/`ZOOMOUT` step path (anchored at the
/// viewport centre); `ALL`/`EXTENTS` dispatches the ordinary `FIT`.
pub const ARGUMENT_SELECTOR: &str = "ZOOM";

/// AutoCAD's `SNAPUNIT <value>`: the session grid-snap spacing sysvar.
///
/// The value is a positive decimal in drawing units and is stored as session
/// state only (no database, history or preference write). It is the spacing the
/// `SNAP` (F9) quantiser uses.
pub const SNAPUNIT_SELECTOR: &str = "SNAPUNIT";

/// One parsed argument of the `ZOOM` selector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomArgument {
    /// A positive finite **scale factor**: `> 1` zooms in, `< 1` zooms out —
    /// the same convention as `ZOOMIN`/`ZOOMOUT` and
    /// [`cad_app::CommandPayload::ZoomAt`]. The zoom is anchored at the
    /// **viewport centre**, exactly like those cursor-less button steps.
    Factor(f64),
    /// `ALL` (the written-out spelling) or `EXTENTS`: frame the whole drawing,
    /// i.e. the [`CommandId::FitDrawing`] path.
    Fit,
}

/// Parse the argument of a `ZOOM` selector: a positive decimal factor, or
/// `ALL` / `EXTENTS` (case-insensitive, surrounding whitespace ignored).
///
/// The numeric grammar is deliberately narrow — ASCII digits with at most one
/// `.` that has digits on **both** sides (`2`, `0.5`, `1.25`, `2.0`). Signs
/// (`-1`, `+2`), exponents (`1e3`), a bare `.`/`1.`, `NaN`/`inf` and zero are
/// all refused with `None`; the caller turns that into the stable refusal from
/// [`invalid_zoom_argument_reason`]. Nothing is clamped and no default factor is
/// invented.
pub fn parse_zoom_argument(argument: &str) -> Option<ZoomArgument> {
    let key = argument.trim();
    if key.eq_ignore_ascii_case("ALL") || key.eq_ignore_ascii_case("EXTENTS") {
        return Some(ZoomArgument::Fit);
    }
    parse_positive_decimal(key).map(ZoomArgument::Factor)
}

/// Parse the argument of a `SNAPUNIT` selector: a positive decimal spacing.
///
/// The grammar is exactly the numeric shape `ZOOM` accepts
/// ([`parse_zoom_argument`]): ASCII digits with at most one `.` that has digits
/// on **both** sides (`25`, `0.5`, `2.5`). Signs (`-5`, `+5`), exponents
/// (`1e3`), a bare `.`/`5.`, `NaN`/`inf`, zero and `-0` are all refused with
/// `None`; the caller turns that into the stable refusal from
/// [`invalid_snap_unit_argument_reason`]. Nothing is clamped, no default spacing
/// is invented and `ALL`/`EXTENTS` mean nothing here.
pub fn parse_snap_unit_argument(argument: &str) -> Option<f64> {
    parse_positive_decimal(argument.trim())
}

/// Split a typed line into an argument-taking selector and its argument.
///
/// `Some((selector, argument))` only when `text` starts with one of
/// [`ARGUMENT_SELECTORS`] followed by one or more spaces/tabs and a non-empty
/// argument (`ZOOM 2`, `  snapunit\t0.5 `); a name-only line (`ZOOM`,
/// `SNAPUNIT`) returns `None` so it keeps the name-only funnel and its existing
/// payload refusal. The argument keeps its internal text: `ZOOM 2 3` reaches
/// [`parse_zoom_argument`] as `2 3` and is refused there rather than being split
/// a second time.
pub fn typed_command_argument(text: &str) -> Option<(&'static str, &str)> {
    let trimmed = text.trim();
    let split = trimmed.find([' ', '\t'])?;
    let (name, rest) = trimmed.split_at(split);
    let argument = rest.trim_start_matches([' ', '\t']).trim_end();
    if argument.is_empty() {
        return None;
    }
    let selector = canonical_command_name(name)?;
    ARGUMENT_SELECTORS
        .contains(&selector)
        .then_some((selector, argument))
}

/// The stable rejection reason for a `ZOOM` argument outside the grammar.
///
/// One wording for one fact: the typed line is echoed verbatim and the accepted
/// grammar is spelled out, so a refusal is never a silent ignore or a clamp.
pub fn invalid_zoom_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (ZOOM needs a positive factor, ALL or EXTENTS)",
        text.trim()
    )
}

/// The stable rejection reason for a `SNAPUNIT` argument outside the grammar.
///
/// Same shape as [`invalid_zoom_argument_reason`]: the typed line is echoed and
/// the accepted grammar (one positive decimal spacing) is named.
pub fn invalid_snap_unit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (SNAPUNIT needs a positive decimal spacing)",
        text.trim()
    )
}

/// Whether `text` is a plain positive decimal number (`digits[.digits]`).
fn parse_positive_decimal(text: &str) -> Option<f64> {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let digits_only = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits_only(whole) {
        return None;
    }
    if let Some(fraction) = fraction {
        if !digits_only(fraction) {
            return None;
        }
    }
    let value: f64 = text.parse().ok()?;
    (value.is_finite() && value > 0.0).then_some(value)
}

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

/// Full-word synonyms and AutoCAD-style abbreviations, always accepted, that map
/// onto a canonical [`COMMAND_TABLE`] selector.
///
/// The single/double-letter rows mirror AutoCAD's default `acad.pgp` command
/// abbreviations **for the commands this build already implements only**: an
/// alias is admitted only when its target is a real selector, so `MA`
/// (`MATCHPROP`) is present now that MATCHPROP exists (and was absent while the
/// command itself was absent). There is no `acad.pgp` file parsing and no
/// user-defined alias support; this table is the whole (curated) subset, and
/// every row is contract-tested to canonicalise onto a [`COMMAND_TABLE`] name,
/// to be unique, and to collide with no canonical name.
///
/// There is no `SNAP` alias row. AutoCAD's `SNAP` is **grid snap** (F9), a real
/// selector of its own ([`CommandId::ToggleSnapGrid`], with the `SNAPUNIT`
/// spacing sysvar); it is a different feature from object snap (`OSNAP`) and
/// from the display grid (`GRID`). Aliasing it onto `ToggleSnap` would silently
/// turn a request for grid snap into an object-snap toggle, and aliasing it onto
/// `ToggleGrid` would silently turn it into a display toggle — neither is grid
/// snap, so no alias exists and the canonical name is the only spelling.
pub const COMMAND_ALIASES: &[(&str, &str)] = &[
    ("EXPORT", "PLOT"),
    ("PRINT", "PLOT"),
    ("SAVEAS", "SAVE"),
    ("QSAVE", "SAVE"),
    ("ZOOM EXTENTS", "FIT"),
    ("ENTER", "CONFIRM"),
    // `DBA`/`DCO` are the conventional AutoCAD abbreviations of the baseline and
    // continue dimension pair. Admitting them is our declared convenience (the
    // AutoCAD command reference documents the full names, not these shorthand
    // forms), and an alias never changes behaviour: it only normalizes a typed
    // name onto the canonical selector (ADR 0017 decision C, P5f).
    // `LAYUNISO` is AutoCAD's "turn layer isolation off", which is exactly what
    // `RESTORELAYERS` does here (drop the session's temporary overrides).
    // `LAYERP` ("previous layer state") has **no** layer-state stack in this build,
    // so it names the same, single restore path rather than inventing a second one.
    // `LAYON` ("turn all layers on") is the same restore reached by its own name.
    ("LAYUNISO", "RESTORELAYERS"),
    ("LAYERP", "RESTORELAYERS"),
    // `LAYON` is AutoCAD's "turn all layers on", which is exactly what the one
    // restore path does here (drop every temporary override). It carries no
    // `CommandId` of its own, like `LAYUNISO`/`LAYERP`.
    ("LAYON", "RESTORELAYERS"),
    ("DBA", "DIMBASELINE"),
    ("DCO", "DIMCONTINUE"),
    // `NEWLAYER` is a **project** word synonym for creating a layer, not an
    // AutoCAD command (AutoCAD reaches layer creation through `LAYER` /
    // `-LAYER`); it is admitted only because `CREATELAYER` itself exists, the
    // same rule every other alias row follows. It is the only non-`acad.pgp`
    // spelling in this table, and it changes nothing but the normalized name.
    ("NEWLAYER", "CREATELAYER"),
    // AutoCAD's default `acad.pgp` single/double-letter abbreviations, listed
    // alphabetically (draw, modify, view, annotate, drafting switches). Only the
    // subset whose target selector exists here: `MA` (MATCHPROP) joined the table
    // when MATCHPROP itself was implemented, and inventing an abbreviation for an
    // absent command is exactly what this table must never do.
    ("A", "ARC"),
    ("AL", "ALIGN"),
    // AutoCAD's `AR` is ARRAY; `ARRAYRECT` is the array command this build
    // implements, so the abbreviation points at the implemented selector.
    ("AR", "ARRAYRECT"),
    ("C", "CIRCLE"),
    ("CHA", "CHAMFER"),
    ("CO", "COPY"),
    ("CP", "COPY"),
    ("DAL", "DIMALIGNED"),
    ("DAN", "DIMANGULAR"),
    ("DCE", "DIMCENTER"),
    ("DDI", "DIMDIAMETER"),
    ("DLI", "DIMLINEAR"),
    ("DRA", "DIMRADIUS"),
    ("E", "ERASE"),
    ("EL", "ELLIPSE"),
    ("EX", "EXTEND"),
    ("F", "FILLET"),
    ("GR", "GRID"),
    ("H", "HATCH"),
    ("L", "LINE"),
    ("M", "MOVE"),
    ("MA", "MATCHPROP"),
    ("MI", "MIRROR"),
    ("O", "OFFSET"),
    ("OR", "ORTHO"),
    ("OS", "OSNAP"),
    ("P", "PAN"),
    ("PL", "PLINE"),
    ("PO", "POINT"),
    ("POL", "POLYGON"),
    ("REC", "RECTANG"),
    ("RO", "ROTATE"),
    ("S", "STRETCH"),
    ("SC", "SCALE"),
    ("SPL", "SPLINE"),
    ("T", "TEXT"),
    ("TR", "TRIM"),
    ("U", "UNDO"),
    ("X", "EXPLODE"),
    ("Z", "ZOOM"),
];

/// The alias table as `(alias, canonical selector)` pairs, resolved through
/// [`canonical_command_name`].
///
/// This is what a host publishes to a frontend (the `cad_command_aliases` IPC):
/// the frontend may offer aliases in its suggestion list without keeping a
/// hand-written mirror of the table. An alias whose target does not resolve is
/// skipped rather than guessed; the alias invariants are contract-tested, so no
/// such row exists.
pub fn alias_pairs() -> Vec<(&'static str, &'static str)> {
    COMMAND_ALIASES
        .iter()
        .filter_map(|(alias, target)| canonical_command_name(target).map(|name| (*alias, name)))
        .collect()
}

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

/// The canonical selector a [`CommandId`] is named by, if it has one.
///
/// This is the reverse of [`COMMAND_TABLE`] and exists for typed entry points
/// that dispatch a `CommandId` directly without going through a text selector,
/// so the command echo still records the same stable machine key the router
/// would. Interaction-only selectors (`DIST`, `ZOOMIN`, …) carry no
/// `CommandId` and are recorded by the selector path itself.
pub fn command_name(id: CommandId) -> Option<&'static str> {
    COMMAND_TABLE
        .iter()
        .find(|(_, candidate)| *candidate == id)
        .map(|(name, _)| *name)
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
