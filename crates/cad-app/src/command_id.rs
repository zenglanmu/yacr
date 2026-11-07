#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Viewer,
    Work,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    OpenDrawing,
    /// Start a new, empty drawing. Host-owned like `OpenDrawing`: the
    /// application layer has no document factory, so it returns `Unsupported`
    /// and a platform host performs the replacement.
    NewDrawing,
    /// Export the active drawing to a vector file (SVG/PDF). Host-owned like
    /// `OpenDrawing`: the application layer has no file chooser, so it returns
    /// `Unsupported` and a platform host performs the export.
    PlotDrawing,
    /// Save the active drawing as a lossy DXF file. Host-owned like
    /// `OpenDrawing`: the application layer has no file chooser, so it returns
    /// `Unsupported` and a platform host performs the save.
    SaveDrawingAs,
    CancelLoading,
    Pan,
    /// Change the viewpoint without moving the model.
    ///
    /// The payload selects the form — `ZoomAt` (factor + cursor), the legacy
    /// `Points` (factor at the viewport centre), `ZoomWindow` (`ZOOM WINDOW`,
    /// exact two-corner fit, 2D orthographic only) and `ZoomPrevious`
    /// (`ZOOM PREVIOUS`, the per-viewport discrete-view stack). The two new
    /// forms deliberately ride **this** command with their own payload, so the
    /// command vocabulary is unchanged.
    ///
    /// Every form is view-only: no database write, no revision bump, no
    /// `cad-history` entry and no `session.generation` bump, so Viewer mode
    /// allows them. Full contract: [`crate::command_id::zoom`].
    Zoom,
    FitDrawing,
    ResetView,
    ToggleLayer,
    /// Atomically publish a nonempty batch of temporary layer overrides.
    /// Allowed in Viewer and Work; no drawing write, reparse or undo entry.
    SetLayerVisibilities,
    RestoreLayers,
    /// `LAYERP`: restore the **previous layer state** — AutoCAD's "previous layer state" stack.
    ///
    /// Payload is [`crate::CommandPayload::None`]; a caller that supplies one is
    /// refused (`layers.previous.invalid_payload`), never silently ignored. The
    /// session keeps a **bounded** history of layer-override snapshots
    /// ([`crate::LAYER_OVERRIDE_HISTORY_CAP`] entries, oldest dropped first); the
    /// pre-change snapshot is pushed by every override mutation that really changes
    /// something (`ToggleLayer`, `SetLayerVisibilities`, `IsolateLayers`,
    /// `LayOff`, `RestoreLayers`), and a mutation that changes nothing pushes
    /// nothing. `LAYERP` **pops** the newest snapshot and applies it: it never
    /// pushes, so repeated `LAYERP` walks back through the recorded states and
    /// cannot ping-pong.
    ///
    /// An empty history is not an error and changes nothing: it reports the
    /// `layers.previous.empty` diagnostic — the same shape as `RestoreLayers`,
    /// which reports "nothing to restore" as a no-op rather than a failure.
    ///
    /// No database write, no revision bump, no history (`cad-history`) entry and
    /// no persisted preference: overrides are session view state. Allowed in
    /// Viewer and Work (a session view operation, not an edit).
    ///
    /// AutoCAD's `LAYERP` walks back one step; `LAYRDO`/`LAYRSTATE` (named layer
    /// states) are **NOT IMPLEMENTED** — this history is an in-memory stack of
    /// session override sets, not a saved layer state.
    LayerPrevious,
    /// `LAYISO`: hide every layer that is not on a selected entity or is not the
    /// current layer, using **session visibility overrides only**.
    ///
    /// Payload is [`crate::CommandPayload::None`]; the selection is the input. The
    /// keep-set is `{ layers of the selected entities } ∪ { current layer }` — the
    /// current layer is included unconditionally because this build already refuses
    /// to make an invisible layer current and refuses to hide the current layer
    /// (`layer.current_off` / `layer.current_frozen`); see
    /// [`crate::layers_isolate`]. When the current layer carried no selected entity
    /// a `layers.isolate.current_layer` diagnostic names it.
    ///
    /// The hides are published as **one atomic batch** through the same validated
    /// path as [`CommandId::SetLayerVisibilities`], so a refusal changes nothing.
    /// No database write, no revision bump, no history entry and no persisted
    /// preference: the drawing's own layer flags stay the authority. When every
    /// layer is already in the keep-set the command writes no override at all and
    /// reports `layers.isolate.noop`.
    ///
    /// `LAYUNISO` and `LAYON` are **aliases of [`CommandId::RestoreLayers`]** (they
    /// clear the overrides); there is no second restore path. `LAYERP` is **not**
    /// an alias: it is [`CommandId::LayerPrevious`], which restores the previous
    /// layer state. Allowed in Viewer and
    /// Work — it is a session view operation, not an edit. An empty selection is an
    /// explicit refusal (`layers.isolate.needs_selection`): isolating nothing would
    /// hide every layer with no way back from the canvas.
    IsolateLayers,
    /// `LAYOFF`: turn **off** every layer the selected entities live on, except
    /// the current layer, using **session visibility overrides only**.
    ///
    /// Payload is [`crate::CommandPayload::None`]; the selection is the input (at
    /// least one entity, refused by name otherwise). It is the sibling of
    /// [`CommandId::IsolateLayers`] and the second half of AutoCAD's layer surgery:
    /// where `LAYISO` keeps the selection's layers and hides the rest, `LAYOFF`
    /// hides exactly the selection's layers.
    ///
    /// The **current layer is never turned off**: this build already refuses to
    /// make an invisible layer current and refuses to turn the current layer off
    /// (`layer.current_off`, see `app_drawing::layer_edit`), and a hidden current
    /// layer would silently receive new geometry. When the selection has entities
    /// *only* on the current layer the command changes nothing and reports
    /// `layers.off.current_layer`.
    ///
    /// The hides are published as **one atomic batch** through the same validated
    /// path as [`CommandId::SetLayerVisibilities`] (session overrides only), so a
    /// refusal changes nothing. No database write, no revision bump, no history
    /// entry. Allowed in Viewer and Work — it is a session view operation, not an
    /// edit.
    ///
    /// `LAYON` is an **alias of [`CommandId::RestoreLayers`]** (it clears the
    /// overrides, which is what turning the layers back on means here); it carries
    /// no `CommandId` of its own. `LAYERP` is a real command
    /// ([`CommandId::LayerPrevious`]), never an alias of this restore.
    LayOff,
    SwitchSpace,
    /// `MODEL`: switch the active drawing space to model space.
    ///
    /// Payload-free and equivalent to the typed `SPACE MODEL` line: both reach
    /// the same session-state switch, which validates the target against the
    /// open drawing and then auto-fits the camera. A caller that supplies a
    /// payload is refused explicitly. No database write, no `cad-history` entry,
    /// no revision bump and no `ChangeSet`, so Viewer mode allows it. The real UI
    /// affordance is the drawing-space strip's Model tab.
    ModelSpace,
    Select,
    /// Select visible model-space pick items without editing the drawing.
    /// Requires `CommandPayload::None`; incomplete picking is explicitly refused.
    SelectAll,
    /// `SELECTSIMILAR`: add every visible model-space pick item whose **geometry
    /// kind** matches a currently selected item to the existing selection.
    ///
    /// Payload is [`crate::CommandPayload::None`]; a non-empty current selection
    /// is required (`selectsimilar.selection_required`) and paper/block active
    /// space is an explicit `Unsupported` refusal. v1 is **type-based only**: the
    /// match is the `SemanticGeometry` discriminant, and attribute similarity
    /// (layer/colour/linetype/lineweight/transparency) is NOT IMPLEMENTED. The
    /// command is a union, never a replacement, and writes nothing: no database
    /// write, revision bump, `cad-history` entry or `ChangeSet`, so Viewer and
    /// Work both allow it. Enumeration reuses the SELECTALL preflight and
    /// visibility filter (cycles/depth/dynamic visibility/opaque geometry are
    /// explicit refusals, never silent truncation).
    SelectSimilar,
    /// Select model-space items in a window/crossing rectangle without editing
    /// the drawing (ADR 0014 B). Allowed in Viewer and Work; requires
    /// `CommandPayload::BoxSelect`.
    SelectInBox,
    Measure,
    /// Confirm the points captured by the active open-ended measurement tool.
    ConfirmMeasurement,
    /// Cancel the active tool without committing anything.
    CancelMeasurement,
    /// `AREA OBJECT`: the **exact area of one closed entity**, computed from that
    /// entity's own stored geometry (our selection-based form of AutoCAD's
    /// `AREA` → `Object`). One selected entity is the whole input.
    ///
    /// Full contract (accepted kinds, every refusal code, the result shape and
    /// the declared divergence from AutoCAD's pick prompt):
    /// [`crate::command_id::area_object`].
    AreaObject,
    Undo,
    Redo,
    Resources,
    Diagnostics,
    /// `AUDIT`: detect-only drawing audit (no repair, read-only; see `app_audit`).
    AuditDrawing,
    SwitchBackend,
    Switch2d3d,
    StandardView,
    /// `PLAN`: display an orthographic view of the XY plane — AutoCAD 2026's
    /// `PLAN` in its default form.
    ///
    /// Payload-free and exactly the **top/plan 2D view**
    /// ([`crate::StandardView::Top`]): it returns the observation to the 2D plan
    /// camera, recording one `ZOOM PREVIOUS` step when it leaves a 3D view and
    /// doing nothing while the viewport is already 2D (the same documented no-op
    /// as the top standard view). Both paths live in `Application::plan_view` /
    /// `Application::apply_standard_view`, so `PLAN` is a second *name* for the
    /// plan observation, never a second view authority.
    ///
    /// AutoCAD's `PLAN` also offers **UCS / World / Current** orientation
    /// choices. This build has **no UCS model at all**, so those options are NOT
    /// IMPLEMENTED and there is no payload field for them: a caller that supplies
    /// any payload is refused explicitly (`InvalidInput`), never silently mapped
    /// onto the world plan.
    ///
    /// View/session state only: no database write, no revision bump, no
    /// `cad-history` entry and no `ChangeSet`, so Viewer mode allows it.
    PlanView,
    Orbit,
    SwitchProjection,
    /// Switch the session between Viewer and Work mode (audit U02).
    ///
    /// Allowed in both directions and in both modes: the mode change itself is
    /// not a Work-only operation, but it cancels any unconfirmed tool. The
    /// command layer still refuses every Work-only command while in Viewer mode.
    SetMode,
    /// Draw a LINE through two payload points into model space. Work-only.
    CreateLine,
    /// Draw a CIRCLE with a centre and an edge point. The radius is the exact
    /// centre→edge distance and must be positive. Work-only.
    CreateCircle,
    /// `CIRCLE 2P`: draw a CIRCLE through two **diameter endpoints**.
    ///
    /// Payload is `Points([first, second])`: the two clicks are the opposite ends
    /// of a diameter, so the stored centre is their midpoint and the radius is
    /// half the distance between them. A non-finite pair or a zero distance
    /// is `circle2p.degenerate` and changes nothing. One entity / one transaction
    /// / one undo step on the active layer. Work-only.
    CreateCircle2P,
    /// `CIRCLE 3P`: draw a CIRCLE through three points.
    ///
    /// Payload is `Points([first, second, third])`. The circumcircle comes from
    /// the **one** construction ARC already uses
    /// ([`crate::draw_tool::arc_parameters`]), so a CIRCLE 3P and an ARC through
    /// the same three points share a centre and radius (the arc's start/sweep are
    /// simply ignored). A non-finite, coincident or collinear triple is
    /// `circle3p.degenerate` and changes nothing. Work-only.
    CreateCircle3P,
    /// `CIRCLE D`: draw a CIRCLE from a centre and a **diameter** point.
    ///
    /// Payload is `Points([center, diameter])`: unlike [`CommandId::CreateCircle`]
    /// the second pick lies on the far side of the circle, so the stored radius
    /// is **half** the centre→pick distance (AutoCAD's Diameter option). A
    /// non-finite pair or a zero distance is `circlediameter.degenerate` and
    /// changes nothing. Work-only.
    CreateCircleDiameter,
    /// Draw an ARC through three points (start, a point on the arc, end) on the
    /// work plane. The three points must be non-collinear and finite; a
    /// degenerate input is an explicit refusal that changes nothing. Work-only.
    CreateArc,
    /// Draw an open LWPOLYLINE through the payload points (at least two, all
    /// finite and not all identical) on the active layer. Straight segments
    /// only (one zero bulge per vertex, `closed == false`); fewer than two
    /// distinct points is an explicit refusal that changes nothing. Work-only.
    CreatePolyline,
    /// Draw an axis-aligned RECTANG from two opposite corners: a closed
    /// four-vertex LWPOLYLINE (one zero bulge per vertex) on the active layer.
    /// Payload is `Points([corner1, corner2])` in click order; the handler
    /// canonicalises the winding and refuses a non-finite, degenerate (zero
    /// width/height) or non-coplanar pair. Work-only.
    CreateRectangle,
    /// Draw a full ELLIPSE by AutoCAD's **axis-endpoint method**: the two
    /// endpoints of the first axis plus a third point giving the distance to the
    /// other axis. Payload is `Points([p1, p2, p3])` in click order; the handler
    /// builds the `SemanticGeometry::Ellipse` (centre, `+Z` normal, longer axis as
    /// `major_axis`, `ratio <= 1`) and refuses a non-finite, degenerate
    /// (coincident endpoints / third point on the first axis) or non-coplanar
    /// triple. The centre method is not implemented. Work-only.
    CreateEllipse,
    /// Draw a closed regular POLYGON **inscribed** in the circle through a centre
    /// pick and a radius point. Payload is `Points([center, radius_point])` in
    /// click order; the side count is the session sysvar
    /// ([`crate::SessionState::polygon_sides`], default 4, range 3..=1024), not a
    /// payload field. The handler builds the closed `n`-vertex
    /// `SemanticGeometry::Polyline` (one zero bulge per vertex) and refuses a
    /// non-finite, zero-radius, non-coplanar or out-of-range input. Work-only.
    CreatePolygon,
    /// Place a single POINT at the payload position. Payload is
    /// `Points([position])` in click order; the handler stores
    /// `SemanticGeometry::Point` and refuses a non-finite position, a
    /// mismatched payload (a `Geometry` or a non-singleton point list) and a
    /// missing active layer. A single point has no coplanarity constraint (it is
    /// trivially coplanar), so the `+Z` refusal is vacuous. AutoCAD documents one
    /// `Specify a point`; **repeat-until-Enter placement is NOT IMPLEMENTED**
    /// (unverified). Work-only.
    CreatePoint,
    /// Draw a fit-point SPLINE through the payload fit points (at least four,
    /// degree 3, clamped chord knots) as one `SemanticGeometry::Spline`. The
    /// handler delegates the interpolation to
    /// `cad_geometry::spline::fit_points_to_cubic_spline` and refuses fewer than
    /// four fit points, non-finite or non-coplanar `+Z` points, a mismatched
    /// payload, a missing active layer, and every degenerate-chord / numerically
    /// singular input the geometry core raises. Work-only.
    CreateSpline,
    /// TEXT: insert one single-line `SemanticGeometry::Text` at the payload
    /// position with the payload's text and height (ADR 0019 F.1, P7a).
    ///
    /// Payload is `Text { position, text, height }`. The interactive tool captures
    /// one insertion point and then the **typed text** through the dedicated
    /// `cad_set_tool_text` channel (text is terminal: submitting it completes the
    /// command, no second Confirm). The height comes from the session typed
    /// parameter ([`crate::draw_tool::ToolParameter::TextHeight`], default
    /// [`crate::draw_tool::TEXT_DEFAULT_HEIGHT`] = 2.5 because the drawing's
    /// `Style` table carries no height), so the capture never invents one; the
    /// rotation comes from [`crate::draw_tool::ToolParameter::TextRotation`]
    /// (degrees, default `0`) and is stored in radians.
    ///
    /// The style is the drawing's `Standard` text style (the `StyleId(0)` entry as
    /// the documented fallback, with its first resource key as the font) and the
    /// alignment is the style default (left/baseline). One entity
    /// (`type_key`/`class_name` = `AcDbText`) is inserted through the shared create
    /// funnel in one transaction / one undo step on the active layer, and the
    /// command refuses an empty/whitespace-only text, an over-1024-character text,
    /// a non-finite position or height, a non-positive height and a missing active
    /// layer. A single-line text has no width factor, oblique angle, character
    /// spacing or background, and MTEXT's columns/background/line-spacing
    /// style/drawing direction are **not** modelled (decision H). Work-only.
    CreateText,
    /// MTEXT: insert one multiline `Text` geometry stored as an `AcDbMText`
    /// entity (the command core; the interactive two-corner tool maps onto it).
    ///
    /// Payload is
    /// `MText { position, width, text, height, rotation, h_align, v_align }`.
    /// Everything the payload carries is used verbatim:
    ///
    /// * `text` — the draft; `\r\n`/`\r`/`\n` are stored as `\P` by the **one**
    ///   policy MTEXT creation and `TEXTEDIT` share, which is exactly why the
    ///   entity is stamped `AcDbMText` (see below);
    /// * `position` — the insertion point (finite);
    /// * `width` — the automatic wrap boundary, the DXF group-41
    ///   `rectangle_width` equivalent ([`cad_domain::wrap_width_from_dxf`] is the
    ///   single sanitization rule). `None` means "no wrap" and stays `None`; a
    ///   present value must be finite and `> 0` and is otherwise **refused** —
    ///   never clamped, never turned into `None`. This is the **two-corner width
    ///   model**: the caller derives the width from the second corner, the
    ///   command does no capture;
    /// * `height` — glyph height in world units (finite, `> 0`, same bound as
    ///   `TEXT`);
    /// * `rotation` — radians (finite);
    /// * `h_align` / `v_align` — the attachment of the run relative to
    ///   `position`.
    ///
    /// **NOT modelled** (no payload field, refused rather than implied; the four
    /// stable reasons the importer already names under `MTEXT_UNMODELLED`):
    /// line-spacing style (`text.line_spacing_style_unsupported`), columns
    /// (`text.columns_unsupported`), background fill
    /// (`text.background_unsupported`) and drawing direction
    /// (`text.drawing_direction_unsupported`).
    ///
    /// The style is the drawing's `Standard` text style (`StyleId(0)` fallback,
    /// first resource key as the font) — the **same** resolution `TEXT` uses. The
    /// stored `type_key` is **`AcDbMText`**, produced by the create funnel's type
    /// key override, so the newline policy, export and `TEXTEDIT` all agree;
    /// MTEXT is never silently downgraded to `AcDbText`. One entity, one
    /// transaction, one undo step on the active layer. Refusals: a wrong payload,
    /// a non-finite position/rotation, empty or whitespace-only text,
    /// over-1024-character text, a non-finite or non-positive height, an
    /// unusable `width`, a missing active layer and Viewer mode. Work-only.
    ///
    /// There is a `DrawToolKind` for MTEXT (2026-10-26): the interactive
    /// two-corner tool captures the first corner (this `position`) and the
    /// opposite corner (which supplies `width`), then the typed text, and maps
    /// onto exactly this payload. The bare `MTEXT` selector therefore **starts
    /// that tool**; a dispatch that carries no payload is still refused by name.
    /// The interactive capture fills the rotation from
    /// [`crate::draw_tool::ToolParameter::TextRotation`] (degrees → radians) and
    /// the attachment from [`crate::draw_tool::ToolParameter::MTextAttachment`] (AutoCAD's `1..=9`
    /// codes), whose defaults reproduce the previously fixed `0.0` / top-left.
    /// Line spacing, columns, background fill, drawing direction and an
    /// in-command editing dialog remain NOT IMPLEMENTED.
    CreateMText,
    /// MLEADER: insert one straight-line multileader with text content from a
    /// two-point capture — the arrow position, then the landing point — followed
    /// by the terminal text step (ADR 0019 decision F.2, P7c).
    ///
    /// Payload is `MLeader { arrow, landing, text }`. The dogleg length, landing
    /// gap, arrowhead size, text height, text style, vertical text attachment,
    /// path type and content kind are **resolved from the drawing's
    /// `MLeaderStyle` table** (`Standard`), never from the payload or a session
    /// parameter: the created entity stores that resolution as its one snapshot
    /// (P7b-1's resolved effective values), so there is no second source.
    ///
    /// The result is **one** `AcDbMultiLeader` — one root, one line, two
    /// vertices, straight, MText content — inserted through the shared create
    /// funnel in one transaction / one undo step on the active layer. The
    /// interactive tool captures the two points and then the typed text through
    /// `cad_set_tool_text` (terminal: submitting it completes the command; no
    /// second Confirm).
    ///
    /// There is **no `Options` sub-prompt** (leader type, content type, maximum
    /// points, angular constraint) and therefore no payload field for any of
    /// them. Text is **required**: there is no path to a content-less
    /// MULTILEADER — content-less leaders are import/display/export only.
    ///
    /// Refusals (nothing written, no transaction): empty/whitespace-only or
    /// over-1024-character text, a non-finite or non-`+Z`-coplanar point pair, a
    /// landing point no farther than the resolved dogleg length, a resolved style
    /// whose path type is not straight (spline/invisible), whose content kind is
    /// block/tolerance, or whose sizes are unusable, a missing `Standard` style,
    /// a missing active layer, and Viewer mode. Work-only.
    MLeader,
    /// TEXTEDIT: replace the text string of an **existing text-bearing entity**
    /// (TEXT / MTEXT / MULTILEADER text; ADR 0020 amendment options A+B, ADR-0021
    /// D4).
    ///
    /// Payload is `TextEdit { target, text }`: the ref resolved at pick time (or
    /// the current single selection for the panel path) plus the replacement
    /// string. The handler re-reads the entity at apply time and refuses by name
    /// when it is gone or no longer a directly addressable text entity (a
    /// content-less MULTILEADER is `textedit.mleader_content_not_text`), so a
    /// stale pick can never be retargeted. Only the string changes — style,
    /// height, rotation, position, alignment and, for a leader, the override
    /// mask/style snapshot/width/attachment and a **rebuilt** display — through
    /// `update_entity` in **one** transaction / one undo step. Compound children
    /// and block-instance members are **not** addressable: a ref with an instance
    /// path or sub-element is refused instead of being reached into. Text
    /// validation matches the `TEXT` channel (empty/whitespace-only and more than
    /// [`crate::draw_tool::MAX_TOOL_TEXT_CHARS`] characters are refusals that
    /// write nothing). An `AcDbText` draft containing a newline is refused with
    /// `textedit.newline_text`; `AcDbMText`/MULTILEADER drafts store newlines as
    /// `\P`. Work-only.
    TextEdit,
    /// DIMLINEAR: draw a **horizontal or vertical** linear dimension through two
    /// extension-line origins and a dimension-line location (ADR 0017 decision C,
    /// P5c). Payload is `Points([first, second, location])` in click order. The
    /// handler decides horizontal vs vertical from the location pick — AutoCAD's
    /// rule: the location determines the direction — and refuses a non-plan
    /// (off-one-`+Z`-plane) triple, coincident origins, an ambiguous direction and
    /// a zero measurement. The dimension is built through the shared synthesis
    /// ([`cad_domain::DimensionGeometry::synthesized`]) from the resolved current
    /// style (`Standard`) and inserted on the active layer in one transaction /
    /// one undo step. Non-associative. Work-only.
    DimLinear,
    /// DIMALIGNED: draw a dimension whose line is **parallel** to the line joining
    /// the two extension-line origins (ADR 0017 decision C, P5c). Payload is
    /// `Points([first, second, location])`; the handler measures the true
    /// origin-to-origin distance and refuses a non-plan triple, coincident origins
    /// and a zero measurement. Same single-entity / one-transaction / active-layer
    /// funnel as [`CommandId::DimLinear`]. Non-associative. Work-only.
    DimAligned,
    /// DIMRADIUS: draw a **radius** dimension for a picked arc/circle (ADR 0017
    /// decision C, P5d). The **only** entry point is an entity pick of an arc or
    /// circle (a fillet arc counts), followed by the dimension-line location.
    /// Payload is `DimRadial { target, location }`. The handler resolves the
    /// entity, places the leader point on the circle along the location pick's
    /// direction, stores the centre in the group-10 slot, and refuses a
    /// non-arc/circle pick, an off-plane entity or location, a degenerate radius
    /// and a location at the centre. Built through the shared synthesis from the
    /// resolved current style (`Standard`) and inserted on the active layer in one
    /// transaction / one undo step. Non-associative. Work-only.
    DimRadius,
    /// DIMDIAMETER: draw a **diameter** dimension for a picked arc/circle (ADR
    /// 0017 decision C, P5d). Same two-step capture as [`CommandId::DimRadius`];
    /// the group-10 slot is the **opposite chord point** and the measurement is the
    /// diameter. Same single-entity / one-transaction / active-layer funnel.
    /// Non-associative. Work-only.
    DimDiameter,
    /// DIMCENTER: create the **non-associative centre mark or centrelines** of a
    /// picked arc/circle as **plain geometry** (ADR 0017 decision C/D.7, P5d) —
    /// ordinary LINE entities, **not** a `DIMENSION`. Payload is
    /// `DimCenter(target)`. The resolved `DIMCEN` drives the result: `> 0` a centre
    /// mark, `< 0` centrelines, `0` nothing (an explicit diagnostic with no
    /// transaction). One transaction / one undo step on the active layer; refuses a
    /// non-arc/circle pick, an off-plane entity, a degenerate radius and a missing
    /// active layer. Work-only.
    DimCenter,
    /// DIMANGULAR: draw an angular dimension (ADR 0017 decision C, P5e). The
    /// interactive tool captures **two straight-line picks** plus the dimension-arc
    /// location; the vertex is the intersection of the two infinite lines (U-8,
    /// pinned against the committed QCAD fixtures) and the location pick selects
    /// one of the four angles. The command also accepts the **three-point**
    /// construction (vertex, two side points, arc location) through
    /// [`crate::AngularDimensionInput::ThreePoint`] — it has no interactive capture
    /// yet because the capture state machine cannot express AutoCAD's
    /// "select a line *or* specify a vertex" first step. Refuses a non-line pick,
    /// parallel/collinear lines, coincident points, an off-plane input, a location
    /// at the vertex and a missing active layer. Built through the shared synthesis
    /// from the resolved current style (`Standard`) and inserted as one
    /// `AcDbDimension` in one transaction / one undo step. Non-associative.
    /// Work-only.
    DimAngular,
    /// DIMORDINATE: draw an ordinate dimension from a feature point and a leader
    /// endpoint (ADR 0017 decision C, P5e). The axis follows the documented auto
    /// rule (a greater Y difference measures the X ordinate; the unstated equal
    /// case is **our** declared choice of Y) unless
    /// [`crate::OrdinateDimensionCommand::axis`] forces one. The measurement is the
    /// feature's datum-relative (world-origin-relative) X or Y coordinate
    /// **magnitude** (P5b: signed ordinates are not modelled). Refuses a zero-length
    /// leader, an off-plane input, non-finite points and a missing active layer.
    /// One transaction / one undo step on the active layer. Non-associative.
    /// Work-only.
    DimOrdinate,
    /// DIMBASELINE: stack **N** linear dimensions off one picked **linear** base
    /// dimension (ADR 0017 decision C, P5f).
    ///
    /// Payload is `DimChain { base, points }`: the base dimension ref resolved at
    /// pick time plus the N picked points in pick order. Row `i` (`i = 1..=N`):
    /// `first = base.first`, `second = P_i`, `rotation = base.rotation`,
    /// `dimension_line = base.location + n̂ · (i · DIMDLI)` where `n̂ = ±rot90(dir)`
    /// is the normal **away from that row's measured geometry** (the sign that
    /// makes `dot(base.location − midpoint(first, second), n̂) > 0`).
    ///
    /// Refusals (named, nothing written, no transaction): a base that is missing,
    /// from another document, an instance-path/sub-element ref, or not a
    /// `DimensionKind::Linear` dimension (aligned/angular/radial/ordinate are
    /// refused by name), zero picked points, a non-finite point, an off-`+Z`-plane
    /// input, a row that measures zero, an undecidable stacking normal (the base
    /// dimension line lies on the measured points — the side is never guessed), a
    /// non-positive row spacing, a missing active layer and Viewer mode.
    ///
    /// All N dimensions are built through the shared synthesis and inserted in
    /// **one transaction / one undo step** on the active layer. Non-associative.
    /// Work-only.
    DimBaseline,
    /// DIMCONTINUE: chain **N** linear dimensions from one picked **linear** base
    /// dimension (ADR 0017 decision C, P5f).
    ///
    /// Payload is `DimChain { base, points }`. Segment 1 is `base.second → P_1`,
    /// segment `i > 1` is `P_{i−1} → P_i`, and every segment keeps
    /// `dimension_line = base.location` (a chain does **not** stack). The pick
    /// order defines the chain direction: a segment whose projection along
    /// `base`'s direction is `<= 0` is refused by name (no folding back). Every
    /// other refusal matches [`CommandId::DimBaseline`], and the N segments are
    /// inserted in one transaction / one undo step. Non-associative. Work-only.
    DimContinue,
    /// HATCH (Work-only): fill the **pre-selected** boundary entities with the
    /// session pattern parameters; one `AcDbHatch` entity in one transaction. Full
    /// contract: [`crate::command_id::hatch`].
    Hatch,
    /// HATCHPOINT: create a HATCH from **one internal point** (project-specific
    /// selector, like SELECTINBOX/GRIPSTRETCH). Payload is
    /// `HatchPoint { seed }`.
    ///
    /// The command layer discovers the **smallest visible closed boundary** that
    /// contains the picked point, includes every nested closed boundary as an
    /// island, and fills with the same session typed channel
    /// ([`crate::draw_tool::ToolParameter::HatchPatternType`] … `HatchStyle`) the
    /// pre-selection HATCH reads.
    ///
    /// Declared v1 limits (the code does not claim more):
    ///
    /// * only **whole visible** `Circle`, full `Ellipse` and closed `Polyline`
    ///   entities are candidates; `LINE`/`ARC` edge chains are **not**
    ///   auto-discovered and the current selection is **not** used;
    /// * only boundaries in the picked point's `+Z` plane (same elevation) are
    ///   considered — other planes, hidden layers and unresolved refs are
    ///   skipped, never projected or invented;
    /// * the seed must be clearly inside or outside: on-edge, no-boundary and
    ///   equal-area-ambiguous picks are explicit refusals (`hatch.boundary_on_edge`,
    ///   `hatch.boundary_not_found`, `hatch.boundary_ambiguous`), never guesses;
    /// * one transaction / one undo step on the active layer, boundary entities
    ///   retained; **no associativity**;
    /// * the existing `HatchGeometry::new` chord/bulge validation blind spot still
    ///   applies to bulged loops.
    ///
    /// Work-only (Viewer mode refuses it at the command layer).
    HatchPoint,
    /// Translate one or more selected entities by a world delta. Exactly one
    /// transaction and one undo step for the whole selection. Work-only.
    MoveEntities,
    /// Trim one LINE/ARC/CIRCLE against LINE/ARC/CIRCLE/ELLIPSE/LWPOLYLINE/SPLINE
    /// boundaries (the honest subset of §3 of `docs/drawing-edit.md`); a SPLINE
    /// boundary is sampled and reported as approximate, SPLINE targets and other
    /// kinds are explicit refusals that change nothing. Work-only.
    TrimEntity,
    /// Extend one LINE/ARC to the nearest boundary crossing beyond the picked end
    /// (same boundary subset as TRIM, sampled SPLINE cuts reported as
    /// approximate); anything else is an explicit refusal that changes nothing.
    /// Work-only.
    ExtendEntity,
    /// `BREAK` one LINE/ARC/CIRCLE/SPLINE between two picked points and keep the
    /// complementary piece(s) (Work-only). Payload is
    /// [`crate::CommandPayload::Break`] `{ target, first, second }`: the first
    /// click is both the entity pick and the first break point, the second click
    /// is the second break point (no `F` override, no multiple-break mode). Picks
    /// are projected onto the target's parameter space and the span between them
    /// removed; the source is retained as the first piece and one fresh entity
    /// (second piece / CIRCLE complement) is inserted when needed.
    ///
    /// Refusals (nothing written): `break.invalid_point`, `break.not_addressable`,
    /// `break.target_missing`, `break.projection_failed`,
    /// `break.pick_outside_span`, `break.zero_span`, `break.whole_curve`,
    /// `break.degenerate_piece`; a LWPOLYLINE (or any other kind) is the shared
    /// `Unsupported`. One transaction / one undo step.
    Break,
    /// CHAMFER two straight lines: trim/extend both to the corner of the two
    /// infinite lines (when both distances are 0) or to the offset points, joining
    /// them with one new LINE (ADR 0016 decision C). The two picks are the two
    /// picked lines; the distances come from the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ChamferDistanceA`]/`B`). One
    /// transaction and one undo step. Work-only.
    ChamferLines,
    /// FILLET two straight lines: trim/extend both to the corner (radius 0) or to
    /// the tangent points of a tangent ARC of the session radius, on the pick side
    /// (ADR 0016 decision B). The two picks are the two picked lines; the radius
    /// comes from the session typed channel
    /// ([`crate::draw_tool::ToolParameter::FilletRadius`]). One transaction and one
    /// undo step. Work-only.
    FilletLines,
    /// `CHAMFER POLYLINE`: the AutoCAD **Polyline mode** of CHAMFER as its own
    /// selection-based selector. Payload is [`crate::CommandPayload::None`] — the
    /// one selected entity is the whole input (the fixed-step capture cannot
    /// express "pick the first object *or* the Polyline option"). The chamfered
    /// entity must be an **open** LWPOLYLINE with **≥ 3** vertices whose every
    /// segment is straight and which is `+Z` planar (v1); every interior vertex is
    /// replaced by its two chamfer split points and the original entity is
    /// rewritten **in place** in one transaction / one undo step. The distances are
    /// the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ChamferDistanceA`]/`B`); `A` measures
    /// along the incoming arm and `B` along the outgoing arm (our declared rule).
    /// A closed ring, an arc segment, a 3D polyline, `A = B = 0`, a distance that
    /// does not fit its segments, a collinear vertex (`CHAMFER POLYLINE` needs a
    /// real corner) and a missing source is an explicit, named refusal that writes
    /// nothing. Work-only.
    ChamferPolyline,
    /// `FILLET POLYLINE`: the AutoCAD **Polyline mode** of FILLET as its own
    /// selection-based selector (same shape as
    /// [`CommandId::ChamferPolyline`]: `CommandPayload::None`, exactly one selected
    /// open straight-segment `+Z` LWPOLYLINE, rewritten in place in one transaction
    /// / one undo step). Every interior vertex becomes two tangent points joined by
    /// an **arc** of the session radius
    /// ([`crate::draw_tool::ToolParameter::FilletRadius`]); the sweep is derived
    /// from the centre angles in the `+Z` plane and stored as the segment bulge.
    /// `R <= 0` is refused **by name**: the two-line `R = 0` corner move has no
    /// polyline equivalent. AutoCAD's radius auto-reduction is **not**
    /// reproduced — a radius that does not fit is a refusal. Work-only.
    FilletPolyline,
    /// `REVCLOUD` (Work-only): selection-based Object mode v1 (`app_drawing::revision_cloud`).
    RevisionCloud,
    /// ARRAYRECT: insert **independent** copies of the current selection on a
    /// rectangular grid, growing up and to the right (ADR 0016 decision D).
    /// Payload is `ArrayRect { refs }`; the row/column counts and spacings are the
    /// session typed channel
    /// ([`crate::draw_tool::ToolParameter::ArrayRows`] … `ArrayColumnSpacing`).
    /// The source objects are **retained** and the base point is the selection's
    /// bounding-box lower-left. Every copy is inserted in one transaction and one
    /// undo step; a total above the explicit cap
    /// ([`crate::app_drawing::MAX_ARRAY_ENTITIES`]) is refused, never truncated.
    /// Associativity is **NOT IMPLEMENTED**. Work-only.
    ArrayRectangular,
    /// ARRAYPOLAR: insert **independent** copies of the current selection around a
    /// captured centre point (ADR 0016 decision D). Payload is
    /// `ArrayPolar { refs, center }`; the item count, fill angle and rotate-items
    /// flag are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ArrayItems`] … `ArrayRotateItems`). The
    /// base point is the selection's bounding-box lower-left; the placement rule
    /// is our declared (unverified) choice. One transaction and one undo step;
    /// associativity is **NOT IMPLEMENTED**. Work-only.
    ArrayPolar,
    /// ALIGN the current selection from two point pairs (ADR 0016 decision F).
    /// Payload is `Align { refs, source1, dest1, source2, dest2 }`; the
    /// `AlignScale` flag is the session typed channel
    /// ([`crate::draw_tool::ToolParameter::AlignScale`]). The transform translates
    /// `source1 → dest1`, rotates by the angle between the two source/destination
    /// directions, and (only when the flag is true) applies a uniform scale of
    /// `|dest2 − dest1| / |source2 − source1|` about `source1`; it is applied to
    /// the whole selection **in place** in one transaction / one undo step. A
    /// non-finite point, a degenerate source or destination pair, an empty
    /// selection or untransformable geometry is an explicit refusal that changes
    /// nothing. Work-only.
    AlignEntities,
    /// STRETCH the entities inside a **crossing window**: a fully enclosed entity
    /// is translated by the displacement; a partially enclosed LINE/LWPOLYLINE
    /// has only its in-window vertices moved (bulge preserved, UNVERIFIED). The
    /// window is always CROSSING (AutoCAD requires one); the enclosure
    /// classification is recomputed from the rectangle and the geometry — never
    /// stored in the selection. Payload is
    /// `Stretch { rect_logical, base, delta }`. One transaction and one undo
    /// step for the whole command; an in-window entity that cannot be handled
    /// fails the whole command (nothing written). Work-only.
    StretchEntities,
    /// Delete every entity in the current selection in one transaction. An
    /// empty selection is an explicit refusal. Work-only.
    EraseEntities,
    /// `JOIN`: merge the current selection into **one** entity (this build's honest subset of AutoCAD's `JOIN`).
    ///
    /// Payload is [`crate::CommandPayload::None`]; the selection is the whole
    /// input. Two shapes are implemented, and nothing else:
    ///
    /// * **all LINE** — every pick must be collinear with the longest one
    ///   (`join.not_collinear`), on one layer (`join.layer_mismatch`) in one space
    ///   (`join.space_mismatch`), with spans that touch or overlap along the common
    ///   direction (`join.not_contiguous`). The result is one LINE spanning
    ///   `min → max` with a **fresh entity id**.
    /// * **all ARC** — every pick must share the reference arc's centre, normal
    ///   direction and radius, its layer and its space, and its sweeps must be
    ///   contiguous in one rotational direction. The result is one ARC with the
    ///   **summed** sweep; a total above one full turn is `join.sweep_overflow`
    ///   (never wrapped) and exactly one turn stays an ARC of `2π` (this build does
    ///   not silently produce a `Circle`).
    ///
    /// Refusals (all before any write): `join.needs_two` (fewer than two distinct
    /// picks), `join.mixed_kinds`, `join.degenerate` (zero-length LINE / zero-sweep
    /// ARC), the four codes above, and [`CadError::Unsupported`] for any other
    /// geometry kind (a capability gap, never a silent partial merge). On success
    /// the sources are deleted and the merged entity inserted in **one transaction
    /// / one undo step**, so an undo restores every source and removes the weld.
    /// The merged entity keeps the lowest draw order of its sources, and its layer,
    /// space and type key come from the sources. Associativity, a polyline/spline
    /// merge, closing a gap, joining across spaces/layers and joining
    /// non-concentric arcs are **NOT IMPLEMENTED**. Work-only.
    Join,
    /// EXPLODE the current selection one level at a time (ADR 0016 decision G): a LWPOLYLINE
    /// becomes straight segments plus ARCs (bulge segments through `bulge_arc_geometry`), a
    /// uniformly scaled INSERT becomes its block members with the placement baked in (nested inserts expanded under a cycle stack
    /// and a depth cap of 32), and a Compound becomes **all** of its children (a
    /// MINSERT yields its Insert instances). Members keep their own
    /// block-definition layers. Refusals (nothing written): a selection with
    /// nothing decomposable; `TEXT`/`SPLINE`/`SHAPE`/`OPAQUE` and mirrored
    /// Arc/Ellipse children via the transform surface; `IMAGE`/`MASK` by policy;
    /// `Dimension`/`MTable`/`MLeader` compounds by `type_key`; a non-uniformly
    /// scaled INSERT; and an INSERT with an untransformable member (the whole
    /// command fails). One transaction and one undo step. Work-only.
    ExplodeEntities,
    /// Clone every entity in the current selection with a fresh id, translating
    /// its geometry by a world delta, in one transaction. An empty selection or
    /// a geometry that cannot be transformed is an explicit refusal. Work-only.
    CopyEntities,
    /// Rotate the current selection about a base point in the XY plane by an
    /// absolute angle (radians). One transaction and one undo step. Work-only.
    RotateEntities,
    /// Scale the current selection uniformly about a base point by a positive
    /// factor. One transaction and one undo step. Work-only.
    ScaleEntities,
    /// Mirror the current selection across the line through two points, keeping
    /// the originals and adding mirrored copies in one transaction. Work-only.
    MirrorEntities,
    /// Capture the current selection into the in-process clipboard. Read-only
    /// (the drawing and history are untouched), so it is allowed in Viewer too.
    CopyToClipboard,
    /// Capture the current selection into the clipboard, then delete it in one
    /// transaction and one undo step. Work-only.
    CutToClipboard,
    /// `COPYBASE`: capture the selection into the clipboard **with a base point**.
    ///
    /// Payload is [`crate::CommandPayload::CopyBase`] `{ refs, base }`; the
    /// interactive form is the one-point [`crate::draw_tool::DrawToolKind::CopyBase`]
    /// capture with a selection (`COPYBASE` → pick a base point), which the host
    /// maps by the active command (see
    /// [`crate::host::HostController`]). Read-only: it clones the payload refs and
    /// opens **no** transaction, so the drawing revision and undo history are
    /// untouched; it records `clipboard_base = Some(base)` so the insertion-point
    /// [`Self::PasteFromClipboard`] can translate by `insertion − base`. Allowed
    /// in Viewer (a read-only capture), like [`Self::CopyToClipboard`]; an empty
    /// `refs` list is an explicit refusal.
    ///
    /// AutoCAD's `COPYBASE` prompt is "Specify base point" after the object
    /// selection, which is exactly this shape. Unlike [`Self::CopyToClipboard`]
    /// this command always records a base — and recording **none** is exactly what
    /// [`Self::CopyToClipboard`] keeps doing.
    CopyToClipboardBase,
    /// `CUTBASE`: [`Self::CopyToClipboardBase`] **and** delete the payload refs.
    ///
    /// Payload is [`crate::CommandPayload::CutBase`] `{ refs, base }`. The capture
    /// is read-only; the delete is one transaction and one undo step through the
    /// same validated delete path ERASE uses (render attributes and
    /// dynamic-visibility membership are restored on undo). The clipboard (entries
    /// **and** base) is committed only **after** the delete succeeded, so a failed
    /// cut cannot destroy the previous clipboard. Work-only.
    CutToClipboardBase,
    /// `PASTECLIP`: insert fresh-id clones of the clipboard entities translated by
    /// `insertion − clipboard base`, in one transaction.
    ///
    /// Payload is [`crate::CommandPayload::PasteClip`] `{ insertion }`; the
    /// interactive form is the one-point [`crate::draw_tool::DrawToolKind::PasteClip`]
    /// capture. The base is the clipboard's recorded
    /// [`crate::SessionState`] base, written by [`Self::CopyToClipboardBase`] /
    /// [`Self::CutToClipboardBase`]; a clipboard captured by
    /// [`Self::CopyToClipboard`] / [`Self::CutToClipboard`] has **none**, so its
    /// base is the world origin and the paste translates by the insertion point
    /// itself. This is AutoCAD's documented `PASTECLIP` behaviour and the reason
    /// `COPYBASE` exists. An empty clipboard is an explicit refusal. Work-only;
    /// attribute/layer/space handling is identical to [`Self::PasteOriginalCoordinates`].
    PasteFromClipboard,
    /// `PASTEORIG`: insert fresh-id clones of the clipboard entities at their
    /// **original** coordinates (the base is ignored).
    ///
    /// Payload is [`crate::CommandPayload::None`]. This is the behaviour
    /// `PASTECLIP` had before it became insertion-point based, kept reachable
    /// under its own AutoCAD name. Exactly one transaction / one undo step; an
    /// empty clipboard is an explicit refusal. Work-only.
    PasteOriginalCoordinates,
    /// `PASTEBLOCK`: turn the in-process clipboard into a **new auto-named block
    /// definition** and insert one reference at the picked insertion point.
    ///
    /// Payload is [`crate::CommandPayload::PasteBlock`] `{ insertion }`; the
    /// interactive form is the one-point
    /// [`crate::draw_tool::DrawToolKind::PasteBlock`] capture (the clipboard is the
    /// input, so no selection is required). Every clipboard entry must be a
    /// **model-space** entity (`pasteblock.not_model_space`); a nested `INSERT`
    /// entry is allowed when its referenced block still exists
    /// (`pasteblock.missing_block`). Member geometry is copied bit-for-bit into
    /// [`cad_domain::SpaceId::Block`]`(new_id)` and the reference carries
    /// `T(insertion) · T(−clipboard base)` (the world origin when the clipboard
    /// has none), exactly like [`Self::PasteFromClipboard`]. The block is named
    /// `PasteBlockN` (the first free suffix), the clipboard is **not** cleared,
    /// and the definition, its members and the reference commit in one
    /// transaction / one undo step. Work-only.
    PasteBlock,
    /// Create an offset copy of a single LINE/CIRCLE source, at the distance and
    /// side given by the picked point. Another geometry kind is an explicit
    /// `Unsupported` refusal. Work-only.
    OffsetEntity,
    /// Set the session's active layer for new drawing entities. Session state
    /// only; no database write. Work-only.
    SetActiveLayer,
    /// `CREATELAYER`: create a new layer table entry (a real database write).
    ///
    /// Payload is [`crate::CommandPayload::CreateLayer`] (the new name). The name
    /// is trimmed and validated by the database: empty/whitespace-only, an
    /// AutoCAD-illegal character and a duplicate (compared
    /// **case-insensitively**) are all explicit refusals that change nothing. One
    /// revision bump, one layer row; the new layer is visible. Layer edits are
    /// **not undoable** (history has no layer patch kind), so no undo entry is
    /// recorded. Work-only.
    CreateLayer,
    /// `RENAMELAYER`: rename an existing layer table entry.
    ///
    /// Payload is [`crate::CommandPayload::RenameLayer`] `{ layer, name }`: the
    /// existing layer addressed **by name** (exact match first, then a unique
    /// case-insensitive one; unknown and ambiguous names are distinct refusals)
    /// and its new name, validated exactly like [`CommandId::CreateLayer`]'s.
    /// Entity references are by id, so a rename retargets nothing. Not undoable;
    /// one revision bump. Work-only.
    RenameLayer,
    /// `DELETELAYER`: delete a layer table entry.
    ///
    /// Payload is [`crate::CommandPayload::DeleteLayer`] `{ layer }` (addressed by
    /// name like [`CommandId::RenameLayer`]). Refused for the default layer `0`
    /// and for a layer still referenced by any entity (the refusal names the
    /// count); an entity's `layer` is never re-pointed behind the user's back.
    /// When the deleted layer is the session's active layer, the session falls
    /// back to layer `0` and reports it in the diagnostics. Not undoable; one
    /// revision bump. Work-only.
    DeleteLayer,
    /// `SETLAYERPROPERTY`: write one attribute of one layer table row.
    ///
    /// Payload is [`crate::CommandPayload::SetLayerProperty`]
    /// `{ layer, key, value }`: the row addressed **by name** (the same
    /// exact-then-unique-case-insensitive rule as [`CommandId::RenameLayer`]),
    /// a key from [`crate::layer_property::SUPPORTED_KEYS`] and that key's
    /// documented value grammar. The one grammar is
    /// [`crate::layer_property`] (a layer records the table's own units:
    /// `lineweight` is 1/100 mm, `transparency` is percent transparent), and
    /// `plotstyle` is a known key that is refused as not implemented.
    ///
    /// A refusal (`layer.unknown`, `layer.ambiguous_name`,
    /// `layer.property.unsupported`, `layer.property.value`,
    /// `layer.linetype_missing`) happens before anything is published, so it
    /// changes no attribute, no revision and no undo entry. A successful write is
    /// one revision bump of the same shape as a create/rename/delete (no entity
    /// change), and **no** undo entry: layer edits are not undoable (history has
    /// no layer patch kind).
    ///
    /// Only `off` / `frozen` change behaviour, through the database's own
    /// `visible == !(off || frozen)` rule; `locked` / `plottable` / the colour /
    /// the linetype / the lineweight / the transparency are stored only. Work-only.
    SetLayerProperty,
    /// `PDMODE <n>`: write the drawing header's `$PDMODE` (the drawing-global
    /// POINT display mode) from the command line. Full contract:
    /// [`crate::command_id::point_style`].
    SetPointDisplayMode,
    /// `PDSIZE <n>`: write the drawing header's `$PDSIZE` (the drawing-global
    /// POINT display size) from the command line. Full contract:
    /// [`crate::command_id::point_style`].
    SetPointDisplaySize,
    /// Flip the idle hover entity preview for this session. Session/display
    /// state only (no database write, no history, no ChangeSet), so it is
    /// allowed in Viewer and Work. It defaults to on (AutoCAD's
    /// `SELECTIONPREVIEW` initial `3`); see
    /// [`crate::SessionState::hover_preview`].
    ToggleHoverPreview,
    /// Flip object snapping for this session (ADR 0014 A.4). Session/display
    /// state only (no database write, no history, no ChangeSet), so it is
    /// allowed in Viewer and Work. It defaults to on; independent of the
    /// persisted `view.overlays.snapHints` preference, which only gates the hint
    /// overlay. See [`crate::SessionState::snap_enabled`].
    ToggleSnap,
    /// Flip the display grid overlay for this session (`GRID`, F7).
    ///
    /// Session/display state only (no database write, no history, no ChangeSet),
    /// so it is allowed in Viewer and Work. It defaults to on; the host draws the
    /// grid only when this switch **and** the persisted `view.overlays.grid`
    /// preference are both on. This is the **display** grid only — grid snap has
    /// its own switch ([`CommandId::ToggleSnapGrid`]). See
    /// [`crate::SessionState::grid_enabled`].
    ToggleGrid,
    /// Flip the **grid snap** switch for this session (`SNAP`, F9).
    ///
    /// Session/display state only (no database write, no history, no ChangeSet),
    /// so it is allowed in Viewer and Work. It defaults to **off**, matching
    /// AutoCAD's initial `SNAP` state. While on, a captured point is quantised to
    /// the nearest multiple of the session snap spacing in X and Y (see
    /// [`crate::snap`]); the spacing itself is the session sysvar set by the typed
    /// `SNAPUNIT <value>` line ([`CommandId::SetSnapSpacing`]). This is neither
    /// object snap ([`CommandId::ToggleSnap`]) nor the display grid
    /// ([`CommandId::ToggleGrid`]), and it is never another selector's alias. See
    /// [`crate::SessionState::snap_grid_enabled`].
    ToggleSnapGrid,
    /// Set the session grid-snap spacing (`SNAPUNIT <value>`).
    ///
    /// Session/display state only: no database write, no history entry, no
    /// ChangeSet and no persisted preference, allowed in Viewer and Work. The
    /// value travels as [`crate::CommandPayload::SnapSpacing`] and must be finite
    /// and strictly positive; anything else is an explicit refusal, never a
    /// silent clamp. A bare `SNAPUNIT` (no argument) is refused by name because
    /// the payload is missing. See [`crate::SessionState::set_snap_spacing`].
    SetSnapSpacing,
    /// Flip the `ORTHO` drafting constraint for this session (`ORTHO`, F8).
    ///
    /// Session/display state only (no database write, no history, no ChangeSet),
    /// so it is allowed in Viewer and Work. It defaults to **off**, matching
    /// AutoCAD's initial `ORTHO` state. While on, the next point entry of a
    /// drawing or measurement capture is constrained to a horizontal or vertical
    /// direction from the tool's previous captured point (see [`crate::ortho`]).
    /// It is applied **after** grid snap and object snap, so it still decides the
    /// final axis; it is **not** grid snap ([`CommandId::ToggleSnapGrid`]). See
    /// [`crate::SessionState::ortho_enabled`].
    ToggleOrtho,
    /// `SETENTITYPROPERTY`: change one named property on **every** currently
    /// selected entity, as one transaction / one undo step.
    ///
    /// Payload is [`crate::CommandPayload::EntityProperty`] `{ key, value }`:
    /// `key` is one of [`crate::property_value::SUPPORTED_KEYS`] and `value`
    /// follows that key's grammar (an existing layer/linetype name, `ByLayer` /
    /// `ByBlock`, `aci:0..=256` / `#RRGGBB`, a canonical millimetre lineweight,
    /// or a transparency percentage `0..=90`). The selection is read from the
    /// session at apply time; a ref with an instance path or sub-element is
    /// refused (`property.not_addressable`) instead of being reached into, and a
    /// value that is not in the grammar is refused, never clamped.
    ///
    /// Entities that already hold the value are not staged; if nothing changes
    /// overall the command creates **no** undo entry (`property.unchanged`).
    /// `text` is not a writable property here (it has its own `TEXTEDIT` path).
    /// Work-only.
    SetEntityProperty,
    /// `MATCHPROP` (Work-only): paint the first selection's properties onto the rest; see [`crate::command_id::match_prop`].
    MatchProp,
    /// `GRIPSTRETCH` (Work-only): apply one grip drag as one entity update; see
    /// [`crate::command_id::grip_stretch`].
    GripStretch,
    /// `DIVIDE` (Work-only): arc-length POINT marks on the selected curve; see
    /// `crate::app_drawing::divide`.
    Divide,
    /// `LENGTHEN` (Work-only): end-anchored change; see `crate::app_drawing::lengthen`.
    Lengthen,
    /// `INSERT` (Work-only): place an existing named block at one picked point;
    /// no block-table write. See `crate::app_drawing::insert_block`.
    InsertBlock,
    /// `SCALETEXT` (Work-only): scale the glyph height of the selected Text/MText; see `crate::app_drawing::scale_text`.
    ScaleText,
    /// `BLOCK` (Work-only): define a named block from the selection at a picked
    /// base point. Full contract: [`crate::command_id::block_define`].
    DefineBlock,
    /// `XLINE` (Work-only): an infinite construction line reduced to the finite
    /// segment the database can store; see
    /// [`crate::command_id::construction_line`].
    CreateXLine,
    /// `RAY` (Work-only): the positive half of an infinite construction line,
    /// stored as a finite clipped segment; see [`crate::command_id::construction_line`].
    CreateRay,
    /// `LIST` (Viewer and Work): report the stored property data of every
    /// selected object as diagnostics. Selection-based (this build reads the
    /// current selection instead of AutoCAD's interactive "Select objects"
    /// prompt), read-only: no database write, revision bump or undo entry. It
    /// reports the entity's space and canonical
    /// colour/linetype/lineweight/transparency; thickness and extrusion are not
    /// modelled and say so. See `crate::app_drawing::inquiry`.
    List,
    /// `STATUS` (Viewer and Work): report drawing statistics, modes and the
    /// model-space extent. Payload-free and document-scoped; read-only. Drawing
    /// creation/edit time, memory usage, limits and the base point are explicitly
    /// not tracked (no drawing-header model). See `crate::app_drawing::inquiry`.
    Status,
    /// `FS` (Viewer and Work): an **Express tool** (command-line only) that adds
    /// every visible model-space entity directly touching the one selected seed
    /// to the current selection (a union, never a replacement). This is AutoCAD's
    /// default `FSMODE=OFF`: no transitive chain (`FSMODE=ON` is NOT
    /// IMPLEMENTED), and crossing and tangent contacts both count. Read-only
    /// except for the session selection; block-instance members are not
    /// enumerated. See `crate::app_drawing::inquiry`.
    Fs,
}

mod capabilities;

pub mod area_object;
pub mod block_define;
pub mod construction_line;
pub mod grip_stretch;
pub mod hatch;
pub mod match_prop;
pub mod point_style;
pub mod zoom;
