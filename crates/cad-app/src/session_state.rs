//! Per-document CAD session state and its recorded backend preference.
//! Split out of the crate root (which keeps `lib.rs` within the size limit).

use cad_db::MeasurementRecord;
use cad_domain::*;
mod drafting_extras;
mod erase_snapshot;
mod toggles;
mod view_extras;
use crate::draw_tool::{
    DrawIntent, DrawPreview, DrawTool, DrawToolKind, ToolParameter, ToolParameterStore, ToolValue,
    DEFAULT_POLYGON_SIDES,
};
use crate::groups::EntityGroups;
use crate::layer_history::LayerOverrideHistory;
use crate::layer_states::LayerStates;
use crate::layers::LayerOverrideSet;
use crate::measure_tool::{MeasurementPreview, MeasurementTool, MeasurementToolKind};
use crate::selection::SelectionSet;
use crate::tool_state::{DrawCapture, ToolSnapshot, ToolState};
use crate::view_states::ViewStates;
use crate::{AppMode, CommandId};
pub use drafting_extras::DraftingExtras;
pub use erase_snapshot::{EraseEntry, EraseSnapshot};
pub use view_extras::{ViewExtras, VisualStyleToken};

/// One entity captured into the in-process clipboard.
///
/// The [`DbEntity`] carries the row; [`cad_db::EntityDisplayState`] carries the
/// per-entity render attributes and dynamic-visibility membership the row does
/// **not**, so a paste preserves exactly what a cut's undo would restore.
#[derive(Debug, Clone)]
pub struct ClipboardEntry {
    pub entity: cad_db::DbEntity,
    pub display: cad_db::EntityDisplayState,
}

/// The session-only "maximized layout viewport" marker (`VPMAX` / `VPMIN`).
///
/// `index` is the zero-based viewport index inside the layout. The command layer
/// uses index `0` because this build has no per-viewport selection model; the
/// marker is a display hint for a host, never drawing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutViewportRef {
    pub layout: LayoutId,
    pub index: usize,
}

pub struct SessionState {
    pub document: DocumentId,
    mode: AppMode,
    pub active_space: SpaceId,
    pub selection: SelectionSet,
    /// Temporary layer visibility. Never the drawing's layer table (F03).
    pub layer_overrides: LayerOverrideSet,
    /// Entity-level **session** hidden set (`HIDEOBJECTS` / `ISOLATEOBJECTS`).
    ///
    /// Session view state only — never a database write, revision bump, undo
    /// entry or persisted preference. Hidden refs are filtered out of picking by
    /// the shared visibility walk and published to the renderer as
    /// `OverlayInputs.hidden` (`FLAG_HIDDEN`). `UNISOLATEOBJECTS` clears it.
    pub hidden_entities: SelectionSet,
    /// Bounded history of layer-override snapshots for `LAYERP`
    /// ([`crate::layer_history`]): session view state only, never persisted.
    pub layer_history: LayerOverrideHistory,
    /// Named layer states (`LAYERSTATE`) plus the `LAYERPMODE` gate.
    pub layer_states: LayerStates,
    /// Session-only named views (`VIEW` / `VIEWGO`), at most 32.
    pub view_states: ViewStates,
    /// Session-only named entity groups (`GROUP` / `UNGROUP` / `GROUPEDIT`),
    /// at most 64. Never a DWG group object.
    pub groups: EntityGroups,
    /// Document-wide `REGEN` stamp. REDRAW uses `generation` instead.
    pub regen_generation: u64,
    /// `VPMAX`/`VPMIN` session marker; never drawing data.
    maximized_viewport: Option<LayoutViewportRef>,
    /// Session-only `VIEWRES` percentage and `VSCURRENT` style token.
    pub view_extras: ViewExtras,
    /// Session-only drafting flags toggled by the extended command batch
    /// (`QTEXT`, `FILL`, `TFRAMES`, `REGENAUTO`, `ISODRAFT`).
    pub drafting_extras: DraftingExtras,

    pub tool: ToolState,
    /// Generation incremented whenever an active tool lifecycle is replaced.
    pub tool_generation: u64,
    pub generation: u64,
    /// Recorded CAD backend preference; the host rebuilds the render session.
    pub backend: BackendChoice,
    /// The session's active layer for new drawing entities (spec F-EDIT).
    ///
    /// Purely session state: it selects where [`CommandId::CreateLine`] /
    /// [`CommandId::CreateCircle`] place a new entity and never mutates the
    /// drawing's layer table. The default is layer `0`, which every drawing is
    /// required to have; a create naming a layer the database does not contain
    /// is an explicit `InvalidInput` rather than a silent fallback.
    pub active_layer: LayerId,
    /// The session-scoped typed tool parameters (ADR 0016 decision A).
    ///
    /// Session/display state only — **never** a persisted config. The general
    /// channel replaces ADR 0015 decision C's narrow `polygon_sides` sysvar: every
    /// parameter starts at its documented default ([`ToolParameter::default_value`],
    /// e.g. `PolygonSides` = 4, AutoCAD's `POLYSIDES` initial value) and is set
    /// only through the typed `SetToolParameter` request (per-parameter range/kind
    /// validation, out of range is an explicit refusal, never a silent clamp). The
    /// values survive a tool switch and a cancel (sysvar semantics), are **not**
    /// reset by a document edit, and return to the defaults only when the session
    /// is rebuilt (a new blank document or a document replacement).
    tool_parameters: ToolParameterStore,
    /// Whether object snapping is active for this session (ADR 0014).
    ///
    /// Session state only: toggling it writes no database and creates no
    /// history entry. The default is `true`; it is deliberately independent of
    /// the persisted `view.overlays.snapHints` preference, which only controls
    /// whether resolved hints are drawn.
    snap_enabled: bool,
    /// Whether the display grid overlay is enabled for this session (`GRID`,
    /// F7).
    ///
    /// Session/display state only: toggling it writes no database, produces no
    /// history entry and schedules no ChangeSet. The default is `true`. It is
    /// deliberately independent of the persisted `view.overlays.grid`
    /// preference: the host draws the grid only when **both** this switch and
    /// that preference are on (see `cad_host`'s render sync). This is the
    /// **display** grid only — grid snap (AutoCAD's `SNAP`/F9) is the separate
    /// [`SessionState::snap_grid_enabled`] switch.
    grid_enabled: bool,
    /// Whether **grid snap** is active for this session (`SNAP`, F9).
    ///
    /// Session/display state only: toggling it writes no database, produces no
    /// history entry and schedules no ChangeSet. The default is **`false`**,
    /// matching AutoCAD's initial `SNAP` off. While on, a captured point is
    /// quantised to the nearest multiple of [`SessionState::snap_spacing`] in X
    /// and Y, before the `ORTHO` constraint — see [`crate::snap`] for the exact
    /// rule, the half-away-from-zero tie rule and the documented
    /// object-snap → grid-snap → ortho order. It never moves a point in Z and it
    /// is neither object snap (`snap_enabled`) nor the display grid
    /// (`grid_enabled`).
    snap_grid_enabled: bool,
    /// The session **snap spacing** in drawing units (AutoCAD's `SNAPUNIT`).
    ///
    /// Session sysvar state only, never persisted and never derived from the
    /// drawing's units. The default is [`crate::snap::DEFAULT_SNAP_SPACING`]
    /// (10.0 drawing units — this build's declared choice, since AutoCAD's
    /// initial value comes from the template and is unknowable here). Only the
    /// typed `SNAPUNIT <value>` path writes it, and only a finite, strictly
    /// positive value is stored: anything else is an explicit refusal, never a
    /// silent clamp. A tool switch, a cancel and a document edit leave it
    /// untouched; only a session rebuild resets it to the default.
    snap_spacing: f64,
    /// Whether the `ORTHO` drafting constraint is active for this session
    /// (`ORTHO`, F8).
    ///
    /// Session state only: toggling it writes no database, produces no history
    /// entry and schedules no ChangeSet. The default is **`false`**, matching
    /// AutoCAD's initial `ORTHO` off. While on, the next point entry (a drawing
    /// capture or a measurement pick) is constrained to a horizontal or vertical
    /// direction from the tool's previous captured point — see
    /// [`crate::ortho`] for the exact rule, the horizontal tie rule and the
    /// documented "applied after object snap and grid snap" ordering. It never
    /// moves a point in Z and it is not grid snap
    /// ([`SessionState::snap_grid_enabled`]).
    ortho_enabled: bool,
    /// Whether idle pointer movement previews the entity under the cursor.
    ///
    /// Session/display state only: toggling it writes no database, produces no
    /// history entry and schedules no ChangeSet. The default is **`true`**, which
    /// matches AutoCAD's `SELECTIONPREVIEW` initial value `3` (on while no
    /// command is active and at "Select objects" prompts). It can be turned off
    /// with the `HOVERPREVIEW` selector. AutoCAD has no hover timer, so the
    /// dwell (`hover_dwell_ms`) remains an explicit deviation, not parity. The
    /// preview never affects object-snap markers, which stay gated on an active
    /// point prompt.
    hover_preview: bool,
    /// How long a resolved hover must stay stable before it is tinted, in
    /// milliseconds (default **20**).
    ///
    /// **Explicit deviation from AutoCAD**, not a parity claim: AutoCAD defines
    /// no dwell (its highlight is immediate and only distance/pickbox-gated).
    /// The owner asked for a deferred highlight so a fast sweep paints nothing,
    /// then lowered the default from 300 ms to 20 ms — about one frame at 60 Hz,
    /// which filters jitter and tiny sweeps without suppressing a fast sweep.
    /// `0` restores the immediate highlight (the documented escape hatch). Read
    /// only when the preview is enabled; object-snap markers are unaffected.
    hover_dwell_ms: u64,
    /// The last structured measurement result produced in this session (F06).
    ///
    /// Set when a measurement is evaluated (auto-completed or explicitly
    /// confirmed). Cleared when the session is rebuilt for new content.
    last_measurement: Option<MeasurementRecord>,
    /// Last measurement choice retained even while no measurement is active.
    selected_measurement_kind: MeasurementToolKind,
    /// In-process entity clipboard (Phase 8).
    ///
    /// Holds clones of the selected entities (with their display state); it is
    /// **never** the OS/system clipboard. It is cleared whenever the document
    /// content is replaced (`HostController::reset_for_new_content` rebuilds the
    /// session), so a paste can never cross documents. `COPYCLIP`/`CUTCLIP` and
    /// `COPYBASE`/`CUTBASE` capture here; `PASTECLIP` inserts fresh-id clones
    /// translated by `insertion − clipboard_base` and `PASTEORIG` inserts them at
    /// their original coordinates.
    pub(crate) clipboard: Vec<ClipboardEntry>,
    /// The base point captured by `COPYBASE`/`CUTBASE`, if any (AutoCAD's "Specify
    /// base point").
    ///
    /// `None` for a clipboard captured by `COPYCLIP`/`CUTCLIP`, and also after a
    /// clear (it is reset together with [`Self::clipboard`]). `PASTECLIP`
    /// translates by `insertion − clipboard_base.unwrap_or(origin)`, so a clipboard
    /// with no base pastes relative to the world origin, exactly like the two
    /// commands' documented behaviour; `PASTEORIG` ignores it entirely.
    pub(crate) clipboard_base: Option<Point3>,
    /// The entities removed by the last ERASE, for `OOPS`.
    ///
    /// `None` until an ERASE commits a delete; a second ERASE replaces it, and
    /// the first OOPS consumes it. Restored entities get **fresh ids** — the
    /// database id allocator never reuses an id — so an OOPS restore is a new
    /// copy, not the same row back.
    last_erase: Option<EraseSnapshot>,
    /// The bounded, session-only multiline style store (`MLSTYLE`).
    ///
    /// Session/display state only: it feeds `MLINE`'s spacing and is never
    /// persisted to the drawing, history or preferences. See
    /// [`crate::ml_style`] for the cap and the exact refusals.
    pub ml_styles: crate::MultilineStyleStore,
    /// The `MULTIPLE` repeat request, if one is armed.
    ///
    /// Session state only. This host has no command repeat loop, so the flag is
    /// recorded and `MULTIPLE` reports the named boundary; no command is
    /// repeated and nothing is written to the drawing.
    multiple: Option<MultipleRepeat>,
}

/// What a `MULTIPLE` invocation asked to repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultipleRepeat {
    /// Bare `MULTIPLE`: repeat the next command.
    NextCommand,
    /// `MULTIPLE <command>`: repeat that canonical selector.
    Command(String),
}

/// CAD backend preference shared by UI, app state and hosts (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    Auto,
    WebGpu,
    WebGl2,
}

impl SessionState {
    pub fn new(document: DocumentId, mode: AppMode) -> Self {
        SessionState {
            document,
            mode,
            active_space: SpaceId::Model,
            selection: SelectionSet::new(),
            layer_overrides: LayerOverrideSet::new(),
            hidden_entities: SelectionSet::new(),
            layer_history: LayerOverrideHistory::new(),
            layer_states: LayerStates::new(),
            view_states: ViewStates::new(),
            groups: EntityGroups::new(),
            regen_generation: 0,
            maximized_viewport: None,
            view_extras: ViewExtras::default(),
            drafting_extras: DraftingExtras::default(),

            tool: ToolState::Idle,
            tool_generation: 0,
            generation: 0,
            backend: BackendChoice::Auto,
            active_layer: LayerId(0),
            tool_parameters: ToolParameterStore::defaults(),
            snap_enabled: true,
            grid_enabled: true,
            snap_grid_enabled: false,
            snap_spacing: crate::snap::DEFAULT_SNAP_SPACING,
            ortho_enabled: false,
            hover_preview: true,
            hover_dwell_ms: 20,
            last_measurement: None,
            selected_measurement_kind: MeasurementToolKind::Distance,
            clipboard: Vec::new(),
            clipboard_base: None,
            last_erase: None,
            multiple: None,
            ml_styles: crate::MultilineStyleStore::default(),
        }
    }

    /// Arm the `MULTIPLE` repeat request (`None` target = the next command).
    pub fn arm_multiple(&mut self, target: Option<String>) {
        self.multiple = Some(match target {
            Some(name) => MultipleRepeat::Command(name),
            None => MultipleRepeat::NextCommand,
        });
    }

    /// The armed `MULTIPLE` request, if any. Session state only.
    pub fn multiple_target(&self) -> Option<&MultipleRepeat> {
        self.multiple.as_ref()
    }

    /// Disarm the `MULTIPLE` request.
    pub fn clear_multiple(&mut self) {
        self.multiple = None;
    }

    /// The session-only maximized layout viewport (`VPMAX`), if any.
    pub fn maximized_viewport(&self) -> Option<LayoutViewportRef> {
        self.maximized_viewport
    }

    /// Set the maximized viewport marker. Session state only.
    pub fn set_maximized_viewport(&mut self, viewport: LayoutViewportRef) {
        self.maximized_viewport = Some(viewport);
    }

    /// Clear the maximized viewport marker (`VPMIN`). Session state only.
    pub fn clear_maximized_viewport(&mut self) {
        self.maximized_viewport = None;
    }

    /// The previous captured point of the active capture, for `ORTHO`.
    ///
    /// `None` for an idle/pick-only tool: the first pick of a tool has no
    /// direction to constrain against (grid snap still applies, it needs no
    /// previous point). A **pending entity-pick** step is excluded as well — an
    /// entity pick is not a point entry and its commit path
    /// ([`SessionState::capture_draw_pick`]) bypasses the constraints, so a
    /// constrained preview there would disagree with the commit.
    fn ortho_previous_point(&self) -> Option<Point3> {
        match &self.tool {
            ToolState::Drawing(capture) => {
                let pending_pick =
                    capture.tool.points().len() < capture.tool.kind().picked_step_count();
                if pending_pick {
                    None
                } else {
                    capture.tool.points().last().copied()
                }
            }
            ToolState::Measuring(tool) => tool.points().last().copied(),
            ToolState::Idle | ToolState::Selecting | ToolState::Panning => None,
        }
    }

    /// Apply the session drafting constraints to one candidate point.
    ///
    /// The full order of a point entry is:
    ///
    /// 1. **object snap** — already applied by the host before the point reaches
    ///    the session ([`crate::host::HostController::capture_tool_point`]);
    /// 2. **grid snap** — [`crate::snap::snap_point`], X and Y quantised to the
    ///    session spacing, Z untouched;
    /// 3. **ORTHO** — [`crate::ortho::ortho_constrain_point`], so the
    ///    orthographic constraint still decides the final axis.
    ///
    /// Capture and preview both call this one helper, so a rubber band can never
    /// disagree with the committed point (and grid snap can never silently
    /// diverge from ortho). The raw point is returned unchanged while both
    /// switches are off or when the tool has no previous captured point.
    ///
    /// The **grip drag** is the third caller (`cad-host`'s `input/grip_drag.rs`):
    /// a dragged handle is another point entry, so it must obey the same two
    /// switches in the same order rather than growing a second constraint solver.
    /// There `previous` is the grip's discarded-original position and `raw` keeps
    /// the handle's own `z`. An invalid spacing needs no special arm here —
    /// [`crate::snap::snap_point`] passes a point through unchanged when the
    /// spacing is not finite and strictly positive, and
    /// [`SessionState::set_snap_spacing`] refuses such a value on write, so an
    /// unusable spacing means the switch has no effect (never a fabricated one).
    pub fn constrained_point(&self, previous: Option<Point3>, raw: Point3) -> Point3 {
        let snapped = crate::snap::snap_point(self.snap_grid_enabled, self.snap_spacing, raw);
        crate::ortho::ortho_constrain_point(self.ortho_enabled, previous, snapped)
    }

    /// Apply the drafting constraints to a preview cursor.
    ///
    /// The preview is a projection of the same pending point entry the next click
    /// commits, so it must show the constrained value rather than the raw cursor.
    /// `None` (no cursor) stays `None`.
    fn constrained_cursor(
        &self,
        previous: Option<Point3>,
        cursor: Option<Point3>,
    ) -> Option<Point3> {
        cursor.map(|raw| self.constrained_point(previous, raw))
    }

    /// Whether idle pointer movement previews the entity under the cursor.
    ///
    /// See [`SessionState::set_hover_preview`] for the AutoCAD-matching default.
    pub fn hover_preview(&self) -> bool {
        self.hover_preview
    }

    /// The current value of one session tool parameter (ADR 0016 decision A).
    ///
    /// Session/display state only; the store is initialised to each parameter's
    /// documented default.
    pub fn tool_parameter(&self, parameter: ToolParameter) -> ToolValue {
        self.tool_parameters.get(parameter)
    }

    /// The session POLYGON side count (ADR 0015 decision C, now carried by the
    /// general channel as [`ToolParameter::PolygonSides`]).
    ///
    /// Session/display state only; the default is [`DEFAULT_POLYGON_SIDES`] (4,
    /// AutoCAD's `POLYSIDES` initial value).
    pub fn polygon_sides(&self) -> u32 {
        match self.tool_parameter(ToolParameter::PolygonSides) {
            ToolValue::Number(sides) => sides as u32,
            // The store validates on write, so `PolygonSides` is always a number;
            // the flag arm is unreachable and falls back to the documented default.
            ToolValue::Flag(_) => DEFAULT_POLYGON_SIDES,
        }
    }

    /// Set one session tool parameter through the single general channel.
    ///
    /// Authorizes the parameter's own create/edit command (all Work-only, so a
    /// Viewer-mode set is an explicit `PermissionDenied`), then validates the
    /// value against the parameter's kind and range with
    /// [`ToolParameter::validate`] — an out-of-range or wrong-kind value is an
    /// explicit `InvalidInput`, never a silent clamp, and never mutates the store.
    /// The value is session state: a tool switch, a cancel and a document edit
    /// leave it untouched; only a session rebuild resets it to the default.
    pub fn set_tool_parameter(
        &mut self,
        parameter: ToolParameter,
        value: ToolValue,
    ) -> CadResult<()> {
        self.authorize(authorizing_command(parameter))?;
        parameter.validate(value)?;
        self.tool_parameters.set(parameter, value);
        Ok(())
    }

    /// **Test-only**: store a parameter value without validation.
    ///
    /// This exists so a contract test can reach a command's own defensive
    /// re-validation of a value the typed channel would already have refused
    /// (today: a HATCH predefined pattern index past the implemented table).
    /// It is `#[cfg(test)]`, so no wire request, host or UI path can inject an
    /// unvalidated value.
    /// **Test-only**: store a parameter value without validation.
    ///
    /// This exists so a contract test can reach a command's own defensive
    /// re-validation of a value the typed channel would already have refused
    /// (today: a HATCH predefined pattern index past the implemented table).
    /// It is `#[cfg(test)]`, so no wire request, host or UI path can inject an
    /// unvalidated value.
    /// The last ERASE snapshot, if any (`OOPS`'s whole input).
    pub(crate) fn last_erase(&self) -> Option<&EraseSnapshot> {
        self.last_erase.as_ref()
    }

    /// Replace (or clear) the last ERASE snapshot.
    ///
    /// A snapshot whose document is not this session's document is dropped
    /// instead of stored, so the invariant "the snapshot belongs to the open
    /// document" cannot be broken by a caller.
    pub(crate) fn set_last_erase(&mut self, snapshot: Option<EraseSnapshot>) {
        self.last_erase = snapshot.filter(|snapshot| snapshot.document == self.document);
    }

    /// **Test-only**: store a parameter value without validation.
    ///
    /// This exists so a contract test can reach a command's own defensive
    /// re-validation of a value the typed channel would already have refused
    /// (today: a HATCH predefined pattern index past the implemented table).
    /// It is `#[cfg(test)]`, so no wire request, host or UI path can inject an
    /// unvalidated value.
    #[cfg(test)]
    pub(crate) fn force_tool_parameter(&mut self, parameter: ToolParameter, value: ToolValue) {
        self.tool_parameters.set(parameter, value);
    }

    /// Set the session POLYGON side count, refusing out-of-range input.
    ///
    /// A thin convenience over [`SessionState::set_tool_parameter`] — the general
    /// store is the single authority for validation and storage (this is **not** a
    /// second entrance: the wire request, the IPC and the session channel are all
    /// the general one). The range is
    /// [`crate::draw_tool::MIN_POLYGON_SIDES`]`..=`[`crate::draw_tool::MAX_POLYGON_SIDES`]
    /// (AutoCAD's `POLYSIDES` legal range); an out-of-range value is an explicit
    /// `InvalidInput`, never a silent clamp. Work-only: the create commands (and
    /// therefore this parameter) are refused in Viewer mode.
    pub fn set_polygon_sides(&mut self, sides: u32) -> CadResult<()> {
        self.set_tool_parameter(
            ToolParameter::PolygonSides,
            ToolValue::Number(f64::from(sides)),
        )
    }

    /// Enable or disable the idle hover entity preview for this session.
    ///
    /// Session/display state only: no database write, no history entry and no
    /// ChangeSet. The default is **`true`**, matching AutoCAD's
    /// `SELECTIONPREVIEW` initial value `3`; `HOVERPREVIEW` turns it off. The
    /// dwell remains a documented deviation (AutoCAD has no hover timer).
    /// Object-snap markers are unaffected: they remain gated on an active point
    /// prompt.
    pub fn set_hover_preview(&mut self, enabled: bool) {
        self.hover_preview = enabled;
    }

    /// Flip the idle hover entity preview and return the new state.
    pub fn toggle_hover_preview(&mut self) -> bool {
        self.hover_preview = !self.hover_preview;
        self.hover_preview
    }

    /// How long a resolved hover must stay stable before it is tinted, in
    /// milliseconds.
    ///
    /// **A deliberate deviation from AutoCAD**, which has no hover timer; see
    /// [`SessionState::set_hover_dwell_ms`]. It is a documented extension, never
    /// presented as AutoCAD parity.
    pub fn hover_dwell_ms(&self) -> u64 {
        self.hover_dwell_ms
    }

    /// Set the hover dwell in milliseconds (`0` = immediate highlight).
    ///
    /// Session/display state only: no database write, no history entry, no
    /// ChangeSet. The default is **20 ms** (lowered from 300 ms by the product
    /// owner; at 60 Hz that is roughly one frame), an explicit product extension —
    /// AutoCAD defines no dwell for `SELECTIONPREVIEW` (the highlight is
    /// immediate, distance-gated only). The dwell is honoured only while the
    /// preview is enabled.
    pub fn set_hover_dwell_ms(&mut self, dwell_ms: u64) {
        self.hover_dwell_ms = dwell_ms;
    }

    pub fn authorize(&self, command: CommandId) -> CadResult<()> {
        if self.mode == AppMode::Viewer && command.requires_work_mode() {
            Err(CadError::PermissionDenied)
        } else {
            Ok(())
        }
    }

    /// Switch modes; an unconfirmed tool is cancelled, never silently committed.
    pub fn switch_mode(&mut self, mode: AppMode) -> CadResult<()> {
        if self.mode != mode {
            self.tool = ToolState::Idle;
            self.bump_tool_generation();
        }
        self.mode = mode;
        Ok(())
    }

    /// Cancel the current tool and any preview, returning to navigation.
    ///
    /// Cancelling never opens a transaction, so a cancelled measurement always
    /// leaves the drawing database and history untouched.
    pub fn cancel_tool(&mut self) -> CadResult<()> {
        if !matches!(self.tool, ToolState::Idle) {
            self.tool = ToolState::Idle;
            self.bump_tool_generation();
        }
        Ok(())
    }

    fn bump_tool_generation(&mut self) {
        self.tool_generation = self.tool_generation.saturating_add(1);
    }

    /// Begin drawing capture and snapshot the current selection identity.
    pub fn begin_draw(&mut self, kind: DrawToolKind) -> CadResult<()> {
        // The command this capture belongs to. It is recorded on the capture (and
        // exposed through [`Self::draw_command`]) because a kind is not always one
        // command: `COPYBASE` and `CUTBASE` share the `CopyBase` capture shape but
        // differ in authorization (Viewer-legal vs work-only) and in the confirmed
        // payload (capture only vs capture **and** delete).
        let command = match kind {
            DrawToolKind::Line => CommandId::CreateLine,
            // XLINE/RAY store one finite clipped segment each; the kind is what
            // distinguishes the two commands (and their stored `type_key`).
            DrawToolKind::XLine => CommandId::CreateXLine,
            DrawToolKind::Ray => CommandId::CreateRay,
            DrawToolKind::Circle => CommandId::CreateCircle,
            // The three CIRCLE options are their own commands.
            DrawToolKind::Circle2P => CommandId::CreateCircle2P,
            DrawToolKind::Circle3P => CommandId::CreateCircle3P,
            DrawToolKind::CircleDiameter => CommandId::CreateCircleDiameter,
            DrawToolKind::Arc => CommandId::CreateArc,
            DrawToolKind::Polyline => CommandId::CreatePolyline,
            // WIPEOUT stores one `Mask` entity (`AcDbWipeout`).
            DrawToolKind::Wipeout => CommandId::Wipeout,
            DrawToolKind::Rectang => CommandId::CreateRectangle,
            DrawToolKind::Ellipse => CommandId::CreateEllipse,
            DrawToolKind::Polygon => CommandId::CreatePolygon,
            // POLYGON CIRCUMSCRIBED is the tangent variant.
            DrawToolKind::PolygonCircumscribed => CommandId::CreatePolygonCircumscribed,
            DrawToolKind::Point => CommandId::CreatePoint,
            DrawToolKind::Spline => CommandId::CreateSpline,
            DrawToolKind::Move => CommandId::MoveEntities,
            DrawToolKind::Trim => CommandId::TrimEntity,
            DrawToolKind::Copy => CommandId::CopyEntities,
            DrawToolKind::Extend => CommandId::ExtendEntity,
            DrawToolKind::Break => CommandId::Break,
            // BREAKATPOINT: entity pick, then a projected break point.
            DrawToolKind::BreakAtPoint => CommandId::BreakAtPoint,
            // The zero-point single-curve edit pair: the selection is the input and
            // the settings are the typed parameter channel.
            DrawToolKind::Divide => CommandId::Divide,
            DrawToolKind::Lengthen => CommandId::Lengthen,
            // SCALETEXT is the same zero-point pre-selection edit: the selection is
            // the input and the factor is the typed parameter channel.
            DrawToolKind::ScaleText => CommandId::ScaleText,
            // BLOCK: the selection snapshot plus one base point and the terminal
            // name define the new block definition and its reference.
            DrawToolKind::BlockDefine => CommandId::DefineBlock,
            DrawToolKind::Rotate => CommandId::RotateEntities,
            DrawToolKind::Scale => CommandId::ScaleEntities,
            DrawToolKind::Mirror => CommandId::MirrorEntities,
            DrawToolKind::Offset => CommandId::OffsetEntity,
            DrawToolKind::Chamfer => CommandId::ChamferLines,
            DrawToolKind::Fillet => CommandId::FilletLines,
            DrawToolKind::ArrayRect => CommandId::ArrayRectangular,
            DrawToolKind::ArrayPolar => CommandId::ArrayPolar,
            DrawToolKind::Align => CommandId::AlignEntities,
            DrawToolKind::Stretch => CommandId::StretchEntities,
            DrawToolKind::DimLinear => CommandId::DimLinear,
            DrawToolKind::DimAligned => CommandId::DimAligned,
            DrawToolKind::DimRadius => CommandId::DimRadius,
            DrawToolKind::DimDiameter => CommandId::DimDiameter,
            DrawToolKind::DimCenter => CommandId::DimCenter,
            DrawToolKind::DimAngular => CommandId::DimAngular,
            DrawToolKind::DimOrdinate => CommandId::DimOrdinate,
            DrawToolKind::DimBaseline => CommandId::DimBaseline,
            DrawToolKind::DimContinue => CommandId::DimContinue,
            DrawToolKind::DimRotated => CommandId::DimRotated,
            DrawToolKind::Hatch => CommandId::Hatch,
            DrawToolKind::HatchPoint => CommandId::HatchPoint,
            // BOUNDARY: one internal point, shared discovery, new polylines.
            DrawToolKind::Boundary => CommandId::Boundary,
            DrawToolKind::Text => CommandId::CreateText,
            // INSERT is point-then-name; the reference is created from existing
            // block metadata, never by writing the block table.
            DrawToolKind::InsertBlock => CommandId::InsertBlock,
            // MTEXT is the multiline create command (one `AcDbMText`).
            DrawToolKind::MText => CommandId::CreateMText,
            DrawToolKind::MLeader => CommandId::MLeader,
            DrawToolKind::TextEdit => CommandId::TextEdit,
            // COPYBASE is a read-only capture (Viewer-legal); CUTBASE is the same
            // capture for the work-only cutting command. The host reads the recorded
            // command to choose `CopyBase` vs `CutBase`.
            DrawToolKind::CopyBase => CommandId::CopyToClipboardBase,
            DrawToolKind::CutBase => CommandId::CutToClipboardBase,
            // PASTECLIP is work-only through its command (the capture itself needs
            // no selection).
            DrawToolKind::PasteClip => CommandId::PasteFromClipboard,
            // PASTEBLOCK writes a new block definition plus its reference; the
            // capture itself needs no selection (the clipboard is the input).
            DrawToolKind::PasteBlock => CommandId::PasteBlock,
        };
        self.authorize(command)?;
        let selection = self.selection.refs().to_vec();
        let tool = DrawTool::new(kind, selection.len());
        self.tool = ToolState::Drawing(Box::new(DrawCapture {
            tool,
            selection,
            command,
        }));
        self.bump_tool_generation();
        Ok(())
    }

    /// Enter selection mode, replacing any active drawing or measurement tool.
    pub fn begin_selecting(&mut self) {
        self.tool = ToolState::Selecting;
        self.bump_tool_generation();
    }

    /// Start or restart the selected measurement algorithm.
    pub fn begin_measurement(&mut self, kind: MeasurementToolKind) -> CadResult<()> {
        self.authorize(CommandId::Measure)?;
        self.selected_measurement_kind = kind;
        self.tool = ToolState::Measuring(MeasurementTool::new(kind));
        self.bump_tool_generation();
        Ok(())
    }

    /// Change the retained measurement choice, restarting capture if active.
    pub fn set_measurement_kind(&mut self, kind: MeasurementToolKind) -> CadResult<()> {
        self.selected_measurement_kind = kind;
        Ok(())
    }

    pub fn selected_measurement_kind(&self) -> MeasurementToolKind {
        self.selected_measurement_kind
    }

    /// Replace any active capture with pan navigation without dispatching a Pan command.
    pub fn begin_pan(&mut self) {
        self.tool = ToolState::Panning;
        self.bump_tool_generation();
    }

    /// Clear only the active draw/measurement cursor, preserving capture and generation.
    pub fn clear_cursor(&mut self) {
        match &mut self.tool {
            ToolState::Drawing(capture) => capture.tool.set_cursor(None),
            ToolState::Measuring(tool) => tool.set_cursor(None),
            ToolState::Idle | ToolState::Selecting | ToolState::Panning => {}
        }
    }

    /// Capture a finite point in the active drawing or measurement tool.
    /// Returns whether an auto-completing measurement became ready.
    ///
    /// For the entity-picking draw kinds (TRIM/EXTEND) this is **not** the
    /// capture path: their steps are entity picks and go through
    /// [`SessionState::capture_draw_pick`] with the ref the host resolved at
    /// capture time. A blind point for such a kind is refused rather than
    /// stored (see [`DrawTool::push_point`]).
    ///
    /// While the session `SNAP` (grid snap) switch is on the point is quantised
    /// to the nearest multiple of the session snap spacing in X and Y, and while
    /// `ORTHO` is on the (already snapped) point is then constrained against the
    /// tool's **previous captured point** (see [`crate::snap`] and
    /// [`crate::ortho`]). The first point of a tool has no previous point, so only
    /// grid snap applies to it. The host resolves object snapping *first* and
    /// passes the snapped point in, so the documented order is object snap →
    /// grid snap → ortho — a deliberate, spelled-out ordering rather than a claim
    /// of AutoCAD's internal order. Only the stored point differs: no database,
    /// history or preference write is involved, and `z` is never moved.
    pub fn capture_point(&mut self, point: Point3) -> CadResult<bool> {
        self.capture_point_with_snap(point, None)
    }

    /// Capture a point together with the object-snap provenance the host resolved
    /// for it.
    ///
    /// This is the capture entry of the host's acquisition funnel
    /// ([`crate::host::HostController::capture_tool_point`]); `snap` is the
    /// candidate whose point the host already substituted, or `None`. The
    /// provenance is stored beside the point so `ID` can report it
    /// ([`crate::PointReport`]).
    ///
    /// **`ID` skips the grid-snap/`ORTHO` constraint.** For every other tool the
    /// constraint chain documented on [`SessionState::capture_point`] applies
    /// unchanged; for `ID` the reported coordinate must be the coordinate the user
    /// aimed at (the snapped point, or the raw pick), so re-quantising it onto the
    /// grid or onto an orthographic axis would report a point the user never
    /// picked. This is our declared rule, recorded in
    /// [`crate::point_report`]'s module docs.
    pub fn capture_point_with_snap(
        &mut self,
        point: Point3,
        snap: Option<crate::point_report::InputSnap>,
    ) -> CadResult<bool> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput("tool point must be finite".into()));
        }
        let identifying = matches!(
            &self.tool,
            ToolState::Measuring(tool) if tool.kind() == MeasurementToolKind::Id
        );
        let point = if identifying {
            point
        } else {
            self.constrained_point(self.ortho_previous_point(), point)
        };
        match &mut self.tool {
            ToolState::Drawing(capture) => {
                capture.tool.push_point(point)?;
                Ok(false)
            }
            ToolState::Measuring(tool) => {
                tool.push_point_with_snap(point, snap);
                Ok(tool.auto_ready())
            }
            _ => Err(CadError::InvalidInput(
                "no point-capture tool is active".into(),
            )),
        }
    }

    /// Capture one entity pick (TRIM/EXTEND, CHAMFER/FILLET, the DIM* radial
    /// family and TEXTEDIT): the raw point and the resolved ref.
    ///
    /// The ref is resolved by the host **at capture time** through the one pick
    /// path; this method only stores the pair atomically, so the entity the user
    /// saw is exactly the one the commit will use. It refuses when no drawing
    /// tool is active or when the active kind is not an entity picker.
    pub fn capture_draw_pick(&mut self, point: Point3, reference: SelectionRef) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Drawing(capture) => capture.tool.push_pick(point, reference),
            _ => Err(CadError::InvalidInput("no drawing tool is active".into())),
        }
    }

    /// Whether the active drawing tool has a pending entity-pick step.
    ///
    /// True only for TRIM/EXTEND while a step is still un-captured; the host
    /// uses it to route a click to the pick path and to know whether to resolve
    /// a cursor-time candidate.
    pub fn draw_requires_entity_pick(&self) -> bool {
        matches!(
            &self.tool,
            ToolState::Drawing(capture) if capture.tool.requires_entity_pick()
        )
    }

    /// The active drawing tool's kind, or `None` when no drawing capture is
    /// active. The host reads it to apply kind-specific pick rules (TEXTEDIT
    /// validates its pick and records the current string).
    pub fn draw_kind(&self) -> Option<DrawToolKind> {
        match &self.tool {
            ToolState::Drawing(capture) => Some(capture.tool.kind()),
            _ => None,
        }
    }

    /// Record the `TEXTEDIT` prefill (the picked entity's current string).
    ///
    /// Presentation only: the edit command re-reads the entity at apply time.
    /// A no-op for any non-drawing tool.
    pub fn set_draw_text_prefill(&mut self, prefill: Option<String>) {
        if let ToolState::Drawing(capture) = &mut self.tool {
            capture.tool.set_text_prefill(prefill);
        }
    }

    /// Record the entity under the cursor for the pending pick step.
    ///
    /// Presentation feedback only (never a commit source); a no-op for any tool
    /// that is not a drawing tool.
    pub fn set_draw_candidate(&mut self, candidate: Option<SelectionRef>) {
        if let ToolState::Drawing(capture) = &mut self.tool {
            capture.tool.set_candidate(candidate);
        }
    }

    /// Move the preview cursor without advancing the lifecycle generation.
    pub fn move_cursor(&mut self, point: Point3) -> CadResult<()> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput("tool cursor must be finite".into()));
        }
        match &mut self.tool {
            ToolState::Drawing(capture) => capture.tool.set_cursor(Some(point)),
            ToolState::Measuring(tool) => tool.set_cursor(Some(point)),
            _ => return Err(CadError::InvalidInput("no cursor tool is active".into())),
        }
        Ok(())
    }

    pub fn draw_preview(&self) -> Option<DrawPreview> {
        match &self.tool {
            ToolState::Drawing(capture) => Some(self.annotate_draw_preview(capture.tool.preview())),
            _ => None,
        }
    }

    /// Attach the session side count to both POLYGON previews and constrain the
    /// pending point entry (grid/ortho) so the rubber band matches the committed
    /// point; a pending entity pick stays raw.
    fn annotate_draw_preview(&self, mut preview: DrawPreview) -> DrawPreview {
        preview.polygon_sides = matches!(
            preview.kind,
            DrawToolKind::Polygon | DrawToolKind::PolygonCircumscribed
        )
        .then_some(self.polygon_sides());
        let pending_pick = preview.points.len() < preview.kind.picked_step_count();
        if !pending_pick {
            preview.cursor =
                self.constrained_cursor(preview.points.last().copied(), preview.cursor);
        }
        preview
    }

    /// Apply the session drafting constraints to a measurement preview cursor.
    ///
    /// The measurement overlay builds its rubber band from
    /// [`MeasurementPreview::cursor`] exactly like the draw overlay, so it is
    /// constrained the same way: measurement points enter through
    /// [`SessionState::capture_point`] and must preview what they will commit
    /// (grid snap, then ortho).
    fn annotate_measurement_preview(&self, mut preview: MeasurementPreview) -> MeasurementPreview {
        // `ID` never applies the drafting constraints (see
        // [`SessionState::capture_point_with_snap`]), so its preview cursor must
        // not be constrained either: the rubber band would otherwise show a point
        // the commit will never report.
        if preview.kind == MeasurementToolKind::Id {
            return preview;
        }
        preview.cursor = self.constrained_cursor(preview.points.last().copied(), preview.cursor);
        preview
    }

    pub fn draw_intent(&self) -> CadResult<DrawIntent> {
        match &self.tool {
            ToolState::Drawing(capture) => capture.tool.intent(),
            _ => Err(CadError::InvalidInput("no drawing tool is active".into())),
        }
    }

    /// Submit the terminal text of a text tool (ADR 0019 decision A.1).
    ///
    /// The authoritative validation lives in [`DrawTool::set_text`] (state,
    /// emptiness, the 1024-character bound), so this is the single entrance used
    /// by the host after `cad_set_tool_text`. A refusal names its reason and
    /// touches nothing; on success the stored text makes the pending command
    /// complete on the next confirmation — the host calls
    /// [`HostController::confirm_draw_traced`] immediately (text is terminal).
    pub fn set_tool_text(&mut self, text: &str) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Drawing(capture) => capture.tool.set_text(text),
            _ => Err(CadError::InvalidInput(
                "tool.text_not_awaiting: no drawing tool is active".into(),
            )),
        }
    }

    /// The first `Esc` while a text prompt is open (ADR 0019 decision A.1).
    ///
    /// Pure focus validation: the tool stays active and still awaits text. A
    /// second `Esc` is the ordinary cancel ([`Self::cancel_tool`]).
    pub fn dismiss_text_focus(&self) -> CadResult<()> {
        match &self.tool {
            ToolState::Drawing(capture) => capture.tool.dismiss_text_focus(),
            _ => Err(CadError::InvalidInput(
                "tool.text_not_awaiting: no drawing tool is active".into(),
            )),
        }
    }

    /// Whether the active tool awaits its terminal text.
    pub fn draw_awaiting_text(&self) -> bool {
        matches!(&self.tool, ToolState::Drawing(capture) if capture.tool.awaiting_text())
    }

    pub fn draw_selection_snapshot(&self) -> Option<&[SelectionRef]> {
        match &self.tool {
            ToolState::Drawing(capture) => Some(&capture.selection),
            _ => None,
        }
    }

    /// The [`CommandId`] of the active drawing capture, or `None` when no drawing
    /// capture is active.
    ///
    /// The single authority the host reads to map a confirmed
    /// [`DrawIntent::CopyBase`] to its payload: `COPYBASE` captures into the
    /// clipboard only, `CUTBASE` captures **and** deletes, and both start the same
    /// capture shape, so the payload can only be chosen from the command.
    pub fn draw_command(&self) -> Option<CommandId> {
        match &self.tool {
            ToolState::Drawing(capture) => Some(capture.command),
            _ => None,
        }
    }

    pub fn tool_snapshot(&self) -> ToolSnapshot {
        match &self.tool {
            ToolState::Idle => ToolSnapshot::Idle,
            ToolState::Selecting => ToolSnapshot::Selecting,
            ToolState::Drawing(capture) => {
                ToolSnapshot::Drawing(self.annotate_draw_preview(capture.tool.preview()))
            }
            ToolState::Measuring(tool) => {
                ToolSnapshot::Measuring(self.annotate_measurement_preview(tool.preview()))
            }
            ToolState::Panning => ToolSnapshot::Panning,
        }
    }

    /// Clear an active tool after successful execution, invalidating stale input.
    pub(crate) fn complete_tool(&mut self) {
        if !matches!(self.tool, ToolState::Idle) {
            self.tool = ToolState::Idle;
            self.bump_tool_generation();
        }
    }
}

mod inspection;
mod tool_authorization;

pub(crate) use tool_authorization::authorizing_command;
