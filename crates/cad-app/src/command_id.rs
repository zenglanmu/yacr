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
    Zoom,
    FitDrawing,
    ResetView,
    ToggleLayer,
    /// Atomically publish a nonempty batch of temporary layer overrides.
    /// Allowed in Viewer and Work; no drawing write, reparse or undo entry.
    SetLayerVisibilities,
    RestoreLayers,
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
    /// `LAYUNISO` and `LAYERP` are **aliases of [`CommandId::RestoreLayers`]** (they
    /// clear the overrides); there is no second restore path. Allowed in Viewer and
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
    /// no `CommandId` of its own.
    LayOff,
    SwitchSpace,
    Select,
    /// Select visible model-space pick items without editing the drawing.
    /// Requires `CommandPayload::None`; incomplete picking is explicitly refused.
    SelectAll,
    /// Select model-space items in a window/crossing rectangle without editing
    /// the drawing (ADR 0014 B). Allowed in Viewer and Work; requires
    /// `CommandPayload::BoxSelect`.
    SelectInBox,
    Measure,
    /// Confirm the points captured by the active open-ended measurement tool.
    ConfirmMeasurement,
    /// Cancel the active tool without committing anything.
    CancelMeasurement,
    Undo,
    Redo,
    Resources,
    Diagnostics,
    SwitchBackend,
    Switch2d3d,
    StandardView,
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
    /// `Style` table carries no height), so the capture never invents one.
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
    /// Rotation, the attachment fields, line spacing, columns, background fill,
    /// drawing direction and an in-command editing dialog remain NOT
    /// IMPLEMENTED (the tool passes `0.0` and the default top-left attachment).
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
    /// HATCH: fill the **pre-selected** boundary entities (ADR 0018 decision D,
    /// P6b). Payload is `Hatch { refs }` — the selection snapshot taken when the
    /// tool started; the tool itself captures **no points** (its arity is the
    /// zero-point exact complement, like ARRAYRECT) and takes no in-command picks.
    ///
    /// Each accepted selected entity becomes exactly one loop in the hatch
    /// plane's OCS 2D frame: a `Circle` (an exact four-quarter-arc bulged loop),
    /// a full `Ellipse` (sampled chords — declared, a true ellipse is not a
    /// circular arc) or a **closed** `Polyline` (its own points and bulges,
    /// including a 3D entity that lies in one plane).
    ///
    /// Selected `Line`/`Arc` entities (in any mix with the kinds above) are
    /// **stitched** into one loop per closed chain: endpoints are matched at the
    /// absolute `1e-6` world-unit tolerance, an edge may be consumed in either
    /// direction, and every `Arc` stays an **exact bulge segment**
    /// (`bulge = tan(sweep / 4)` — never sampled). A node where more than two
    /// edges meet is refused as `hatch.boundary_branch` (no pairing is guessed),
    /// edges left outside every loop as `hatch.boundary_open` (naming how many),
    /// and a zero-length `Line` / zero-sweep `Arc` as
    /// `hatch.boundary_degenerate`. An open polyline, spline, text, block,
    /// dimension or any other kind is an explicit refusal that opens no
    /// transaction; so is a selection that is not coplanar with a **single
    /// consistent normal**.
    ///
    /// The fill, spacing, angle (degrees on the channel), double flag and style
    /// are the session typed channel ([`crate::draw_tool::ToolParameter::HatchPatternType`]
    /// … `HatchStyle`), read here so the session stays the single authority. A
    /// user-defined pattern is synthesized from angle/spacing/double exactly like
    /// the import path (one definition row at offset `(0, 1)` with the spacing as
    /// the pattern scale) and recorded as
    /// [`cad_domain::HatchPatternType::UserDefined`]; a solid fill is recorded as
    /// `Predefined` (`76=1`, matching the committed SOLID fixtures). Pattern type
    /// code `2` (predefined) takes the `hatchPattern` **1-based** index into the
    /// implemented `ANSI31`/`ANSI32`/`ANSI37` table, records `Predefined` (`76=1`)
    /// and refuses an index outside that table by name.
    ///
    /// The display is derived by the unique constructor `HatchGeometry::new`,
    /// which rejects the ADR A.3 creation-path defects (self-intersecting,
    /// touching/shared vertex, duplicate, partially overlapping, unclosed,
    /// degenerate, non-finite) as a **whole-command** refusal. One entity
    /// (`type_key`/`class_name` = `AcDbHatch`) is inserted through the shared
    /// create funnel in one transaction / one undo step on the active layer.
    /// Associativity, boundary-from-an-internal-point, self-intersecting boundary
    /// loops (refused, never trimmed or repaired), custom (`76=2`) pattern names
    /// and `.pat` loading, a pattern picker dialog, gradient creation, a live
    /// preview and a per-span dash phase reset are **NOT IMPLEMENTED**. Work-only.
    Hatch,
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
    /// `JOIN`: merge the current selection into **one** entity (this build's
    /// honest subset of AutoCAD's `JOIN`).
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
    /// EXPLODE the current selection one level at a time (ADR 0016 decision G):    /// a LWPOLYLINE becomes straight segments plus ARCs (bulge segments through
    /// `bulge_arc_geometry`), a uniformly scaled INSERT becomes its block members
    /// with the placement baked in (nested inserts expanded under a cycle stack
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
    /// Insert fresh-id clones of the clipboard entities at their original
    /// coordinates in one transaction (PASTEORIG). Work-only.
    PasteFromClipboard,
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
    /// POINT display mode) from the command line.
    ///
    /// Payload is [`crate::CommandPayload::PointDisplayMode`] `i16` and is stored
    /// **raw** in [`cad_db::DrawingDatabase`]: `0` dot, `1` none, `2` plus,
    /// `3` cross, `4` vertical tick, plus the `32`/`64`/`96` surrounding-shape
    /// bits and any value this build cannot express. Interpretation (including the
    /// `overlay.point-mode-*` diagnostics) stays in the overlay; the database never
    /// invents a marker shape and never rewrites the header.
    ///
    /// One real write is **one revision bump** and no entity change (published as
    /// a `ChangeSet` through the ordinary "replace the document's
    /// `Arc<DrawingDatabase>`" path), which is exactly what refreshes the display:
    /// the render scene caches the POINT display style per base revision. Writing
    /// the already stored value is an authoritative no-op (`before == after`, no
    /// bump, a `(already stored, no change)` diagnostic).
    ///
    /// **No undo entry** — there is no header-variable patch kind in
    /// `cad-history`, so this command deliberately records no history step rather
    /// than fabricating an entity patch. That is a documented exception to
    /// "one command = one undo step": UNDO cannot restore the previous mode.
    ///
    /// **Not a round-trip claim**: the DXF exporter does write `$PDMODE` (see
    /// [`crate::app_drawing`]), so a SAVE→OPEN cycle keeps the value through our
    /// own writer + reader — internal consistency only, never a vendor
    /// compatibility claim. A source drawing's value is still overwritten by this
    /// write, exactly as AutoCAD's `PDMODE` does.
    ///
    /// Work-only: it writes the document, so Viewer mode refuses it through
    /// [`CommandId::requires_work_mode`] + [`crate::SessionState::authorize`]
    /// (`CadError::PermissionDenied`). A bare `PDMODE` (no argument) is refused by
    /// name — a missing payload is an explicit `InvalidInput`, never a fabricated
    /// default. See [`crate::app_drawing`]'s `point_style` module.
    SetPointDisplayMode,
    /// `PDSIZE <n>`: write the drawing header's `$PDSIZE` (the drawing-global
    /// POINT display size) from the command line.
    ///
    /// Payload is [`crate::CommandPayload::PointDisplaySize`] `f64`, stored
    /// **raw**: `> 0` an absolute drawing-unit size, `0` the viewport-relative DWG
    /// default, `< 0` a percentage of the viewport height. A non-finite value is
    /// refused as `InvalidInput` by both this command and the database write, so
    /// no value the display layer would have to sanitise can be stored.
    ///
    /// One real write is one revision bump with no entity change (the display
    /// refresh signal, see [`CommandId::SetPointDisplayMode`]); the stored value is
    /// a no-op (`(already stored, no change)`), and **no undo entry** is recorded
    /// (no header patch kind — a documented exception to "one command = one undo
    /// step"). The DXF exporter writes `$PDSIZE`, so a SAVE→OPEN cycle keeps it
    /// (our writer + reader, internal consistency only).
    ///
    /// Work-only (a document mutation; Viewer is `PermissionDenied` by the same
    /// rule as [`CommandId::SetPointDisplayMode`]). A bare `PDSIZE` is refused by
    /// name.
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
    /// `MATCHPROP`: paint the display properties of the current selection's
    /// **first** entity onto every other selected entity — the selection-based
    /// form of AutoCAD's property painter.
    ///
    /// Payload is [`crate::CommandPayload::None`]: the selection is the input,
    /// read from the session at apply time. The selection's **own order** (the
    /// pick order [`crate::SelectionSet`] preserves, never re-sorted by id) is
    /// the contract: the first selected entity is the **source**, every other
    /// selected entity is a **destination**. Fewer than two selected entities is
    /// an explicit refusal (`matchprop.needs_two`) — there is no
    /// source-without-destination form.
    ///
    /// Exactly the five writable display properties are transferred: `layer`,
    /// `color`, `linetype`, `lineweight`, `transparency` (the set
    /// [`crate::property_value::SUPPORTED_KEYS`] accepts, projected and
    /// re-parsed through that module's one grammar). **Not transferred** — and
    /// never silently approximated: MATCHPROP's per-property *Settings* subset
    /// (which properties to copy), text style/height, dimension style, hatch
    /// pattern and every other non-display value, and the entity's own linetype
    /// scale (not one of the five keys, so a named destination linetype keeps the
    /// destination's scale). Source and destination **kinds may differ** — that
    /// is the point — as long as every value is valid for the destination.
    ///
    /// Every refusal happens before any mutation (zero database change, zero
    /// undo entry): `matchprop.needs_two`; `property.not_addressable` /
    /// `property.target_missing` for a source **or** destination ref that is a
    /// block-instance member, a sub-element, another document's, or missing (the
    /// property-write rule, reused); and `matchprop.source_value` naming the
    /// source entity and the property when the source's stored value has no
    /// writable form (the parser's own `property.*` code is preserved).
    ///
    /// A destination that already matches contributes no change and is not
    /// staged. If **every** destination already matches, the command reports
    /// `matchprop.unchanged` with **no** transaction and no empty undo entry;
    /// otherwise all changed entities share one transaction / one undo step.
    /// Work-only.
    MatchProp,
    /// `GRIPSTRETCH`: apply **one** grip drag (one handle of one entity) as one
    /// entity update, one transaction / one undo entry.
    ///
    /// Payload is [`crate::CommandPayload::GripStretch`] (boxed
    /// [`crate::GripStretchCommand`] `{ id, to }`, where `id` is a
    /// [`crate::grips::GripId`]). It is the **payload path of a pointer
    /// gesture**: `cad_host` produces it, and only from the release of an armed
    /// grip drag — its pointer routing aims at a handle of the current selection
    /// at press time, tracks the drag and dispatches exactly one `GripStretch` on
    /// release (a drag that did not move the handle dispatches nothing, and a
    /// cancel drops the armed state without writing). This crate keeps no grip
    /// gesture state and never fabricates one; 3D grip editing is NOT
    /// IMPLEMENTED, and this doc records the code path, not a runtime or visual
    /// acceptance. The bare `GRIPSTRETCH` selector carries no handle, so it is
    /// refused explicitly (no fabricated drag).
    ///
    /// The handler re-reads `id.entity` (a missing entity is `grip.target_missing`
    /// and a non-model-space one reuses `property.not_addressable`), then
    /// re-validates the handle against the current geometry via
    /// [`crate::grips::apply_grip`]. Every refusal — `grip.stale_handle`,
    /// `grip.zero_displacement`, `grip.unsupported_kind`,
    /// `grip.non_finite_target` — happens **before** the transaction is opened,
    /// so nothing is written and no empty undo entry exists. It records history
    /// (unlike the layer-table writes) and never routes through
    /// [`CommandId::StretchEntities`]. Work-only.
    GripStretch,
}

impl CommandId {
    pub fn requires_work_mode(self) -> bool {
        matches!(
            self,
            Self::Measure
                | Self::ConfirmMeasurement
                | Self::Undo
                | Self::Redo
                | Self::CreateLine
                | Self::CreateCircle
                | Self::CreateArc
                | Self::CreatePolyline
                | Self::CreateRectangle
                | Self::CreateEllipse
                | Self::CreatePolygon
                | Self::CreatePoint
                | Self::CreateSpline
                | Self::CreateText
                | Self::CreateMText
                | Self::MLeader
                | Self::TextEdit
                | Self::DimLinear
                | Self::DimAligned
                | Self::DimRadius
                | Self::DimDiameter
                | Self::DimCenter
                | Self::DimAngular
                | Self::DimOrdinate
                | Self::DimBaseline
                | Self::DimContinue
                | Self::Hatch
                | Self::MoveEntities
                | Self::TrimEntity
                | Self::ExtendEntity
                | Self::ChamferLines
                | Self::FilletLines
                | Self::ArrayRectangular
                | Self::ArrayPolar
                | Self::AlignEntities
                | Self::StretchEntities
                | Self::EraseEntities
                | Self::Join
                | Self::ExplodeEntities
                | Self::CopyEntities
                | Self::RotateEntities
                | Self::ScaleEntities
                | Self::MirrorEntities
                | Self::CutToClipboard
                | Self::PasteFromClipboard
                | Self::OffsetEntity
                | Self::SetActiveLayer
                | Self::CreateLayer
                | Self::RenameLayer
                | Self::DeleteLayer
                | Self::SetLayerProperty
                | Self::SetPointDisplayMode
                | Self::SetPointDisplaySize
                | Self::SetEntityProperty
                | Self::MatchProp
                | Self::GripStretch
        )
    }
}
