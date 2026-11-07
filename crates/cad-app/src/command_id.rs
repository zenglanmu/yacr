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
    /// Change the viewpoint without moving the model; the payload selects the
    /// form. View-only, so Viewer mode allows it. Full contract:
    /// [`crate::command_id::zoom`].
    Zoom,
    FitDrawing,
    ResetView,
    ToggleLayer,
    /// Atomically publish a nonempty batch of temporary layer overrides.
    /// Allowed in Viewer and Work; no drawing write, reparse or undo entry.
    SetLayerVisibilities,
    RestoreLayers,
    /// `LAYERP`: restore the **previous layer state**.
    ///
    /// Payload is [`crate::CommandPayload::None`]; a caller that supplies one is
    /// refused (`layers.previous.invalid_payload`), never silently ignored. The
    /// session keeps a **bounded** history of layer-override snapshots (cap
    /// [`crate::LAYER_OVERRIDE_HISTORY_CAP`], oldest dropped first), pushed by
    /// every override mutation that really changes something; `LAYERPMODE OFF`
    /// suppresses those pushes. `LAYERP` **pops** the newest snapshot and applies
    /// it: it never pushes, so repeated presses walk back monotonically.
    ///
    /// An empty history is not an error: it reports `layers.previous.empty` and
    /// changes nothing. No database write, revision bump, history entry or
    /// preference: overrides are session view state, so Viewer and Work both
    /// allow it. Named layer states are `LAYERSTATE`, a separate command.
    LayerPrevious,
    /// `LAYISO`: hide every layer the selection is not on (plus the current
    /// layer) using session visibility overrides only. Full contract:
    /// [`crate::command_id::layers::IsolateLayersCommand`].
    IsolateLayers,
    /// `LAYOFF`: turn off exactly the selection's layers (the current layer
    /// excepted) using session visibility overrides only. Full contract:
    /// [`crate::command_id::layers::LayOffCommand`].
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
    /// `WIPEOUT`: closed polygon mask; see [`crate::command_id::draw_variants`].
    Wipeout,
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
    /// pick and a radius point; the side count is the session sysvar
    /// ([`crate::SessionState::polygon_sides`], 3..=1024), not a payload field.
    /// The handler builds the closed `n`-vertex LWPOLYLINE and refuses a
    /// non-finite, zero-radius, non-coplanar or out-of-range input. Work-only.
    CreatePolygon,
    /// `POLYGON CIRCUMSCRIBED`: see [`crate::command_id::draw_variants`].
    CreatePolygonCircumscribed,
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
    /// `TEXT`: one point + terminal text; rotation from `textRotation`; single
    /// line only. Full contract: `crate::app_drawing::drawing_create`.
    CreateText,
    /// `MTEXT`: two-corner multiline text creation; width `None` = no wrap,
    /// `Some(w)` must be finite and `> 0`; one transaction/one undo. Full
    /// contract: `docs/compatibility.md` and `crate::app_drawing::mtext_create`.
    CreateMText,
    /// `MLEADER`: arrow -> landing -> text; single cluster/line; text required.
    /// Full contract: `crate::app_drawing::mleader` and `docs/compatibility.md`.
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
    /// DIMCONTINUE: see [`crate::command_id::dimension_chain`].
    DimContinue,
    /// DIMROTATED (Work-only): a linear dimension whose dimension line is rotated
    /// to the session angle; see [`crate::command_id::dimension_extra`].
    DimRotated,
    /// DIMSPACE (Work-only): equal automatic re-spacing of the selected linear
    /// dimensions; see [`crate::command_id::dimension_extra`].
    DimSpace,
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
    /// The shared internal-point discovery and every declared v1 limit (whole
    /// visible `Circle`/full `Ellipse`/closed `Polyline` only, `+Z` plane only,
    /// on-edge/no-boundary/ambiguous refusals, one transaction, no associativity)
    /// are documented once in [`crate::command_id::boundary`]. Work-only (Viewer
    /// mode refuses it at the command layer).
    HatchPoint,
    /// BOUNDARY (Work-only): one closed, non-associative polyline per boundary
    /// loop discovered from one internal point, on the active layer, in one
    /// transaction. Source entities are retained; `-BOUNDARY` options are NOT
    /// IMPLEMENTED. Full contract: [`crate::command_id::boundary`].
    Boundary,
    /// HATCHGENERATEBOUNDARY: see [`crate::command_id::hatch_generate_boundary`].
    HatchGenerateBoundary,
    /// HATCHTOBACK: see [`crate::command_id::hatch_to_back`].
    HatchToBack,
    /// GRADIENT: see [`crate::command_id::gradient`].
    Gradient,
    /// HATCHEDIT: see [`crate::command_id::hatchedit`].
    HatchEdit,
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
    /// `JOIN`: weld the current selection into one polyline; full contract:
    /// `crate::app_drawing::join`.
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
    /// `SETLAYERPROPERTY`: write one validated layer-table property; layer
    /// writes are not undoable. Full contract: `crate::command_id::layers`.
    SetLayerProperty,
    /// The layer-selection run (`LAYCUR`…`SETBYLAYER`); see [`crate::app_drawing::layer_commands`].
    LayCur,
    LayMCur,
    LayFrz,
    LayThw,
    LayLck,
    LayUlk,
    SetByLayer,
    /// `LAYMCH` / `-LAYMCH`: move the selection to the first pick's layer.
    LayMCh,
    /// `LAYMRG` / `-LAYMRG`: merge the selection's layers into the active layer.
    LayMrg,
    /// `LAYDEL` / `-LAYDEL`: delete the selection's layers and their model entities.
    LayDel,
    /// `LAYERSTATE <option>`: bounded named layer states (`SAVE`/`RESTORE`/`LIST`/`DELETE`).
    LayerState,
    /// `LAYERPMODE [ON|OFF]`: gate whether override mutations record `LAYERP` snapshots.
    LayerPmode,
    /// `LTSCALE <value>`: write the drawing-global linetype scale (`$LTSCALE`).
    SetLinetypeScale,
    /// `LAYWALK <name>[,<name>...]`: keep only the named layers plus the active layer visible.
    LayWalk,
    /// `PDMODE <n>`: write the drawing-global POINT display mode `$PDMODE`; full
    /// contract in [`crate::command_id::point_style`].
    SetPointDisplayMode,
    /// `PDSIZE <n>`: write the drawing-global POINT display size `$PDSIZE`; full
    /// contract in [`crate::command_id::point_style`].
    SetPointDisplaySize,
    /// Flip the idle hover entity preview for this session. Session / display state
    /// only (no database write/history/`ChangeSet`), allowed in Viewer and Work;
    /// defaults on (`SELECTIONPREVIEW` 3). See [`crate::SessionState::hover_preview`].
    ToggleHoverPreview,
    /// Flip object snapping for this session (ADR 0014 A.4). Session / display
    /// state only (no database write, no history, no `ChangeSet`), so Viewer and
    /// Work both allow it. Defaults on; independent of the persisted
    /// `view.overlays.snapHints` preference, which only gates the hint overlay.
    /// See [`crate::SessionState::snap_enabled`].
    ToggleSnap,
    /// Flip the display grid overlay for this session (`GRID`, F7). Session /
    /// display state only (no database write, no history, no `ChangeSet`), so
    /// Viewer and Work both allow it. Defaults on; the host draws the grid only
    /// when this switch **and** the persisted `view.overlays.grid` preference are
    /// both on. This is the **display** grid only — grid snap has its own switch
    /// ([`CommandId::ToggleSnapGrid`]). See [`crate::SessionState::grid_enabled`].
    ToggleGrid,
    /// Flip the **grid snap** switch for this session (`SNAP`, F9). Session /
    /// display state only (no database write, no history, no `ChangeSet`), so
    /// Viewer and Work both allow it. Defaults **off** (AutoCAD's initial state).
    /// While on, a captured point is quantised to the nearest multiple of the
    /// session snap spacing in X and Y ([`crate::snap`]); the spacing is the
    /// `SNAPUNIT <value>` sysvar ([`CommandId::SetSnapSpacing`]). Neither object
    /// snap ([`CommandId::ToggleSnap`]) nor the display grid
    /// ([`CommandId::ToggleGrid`]), and never another selector's alias. See
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
    /// `LIST` / `STATUS` / `FS`: read-only inquiry; see `crate::app_drawing::inquiry`.
    List,
    Status,
    Fs,
    /// `MASSPROP`: read-only mass properties; full contract in
    /// [`crate::command_id::massprop`].
    MassProp,
    /// `MEASUREGEOM`: the inquiry router; full contract in
    /// [`crate::command_id::measuregeom`].
    MeasureGeom,
    /// `REVERSE`: see [`crate::command_id::reverse`].
    Reverse,
    /// `PEDIT` (Work-only): one declared v1 polyline option; see
    /// [`crate::command_id::pedit`].
    Pedit,
    /// `OOPS`: see [`crate::command_id::oops`].
    Oops,
    /// `OVERKILL`: see [`crate::command_id::overkill`].
    Overkill,
    /// `BREAKATPOINT`: see [`crate::command_id::break_at_point`].
    BreakAtPoint,
    /// `TORIENT` / `TCASE` / `TXT2MTXT`: see
    /// [`crate::command_id::text_commands`].
    Torient,
    TCase,
    Txt2MText,
    /// `TEXTTOFRONT` (Work-only): rewrite the selected text/leader/dimension
    /// draw order above every non-eligible model-space object. Payload `None`;
    /// the current selection is the input. Full contract:
    /// [`crate::command_id::texttofront`].
    TextToFront,
    /// `JUSTIFYTEXT` / `TJUST` (Work-only): set only `h_align`/`v_align` of the
    /// selected Text/MText. Full contract:
    /// [`crate::command_id::text_batch::JustifyTextCommand`].
    JustifyText,
    /// `TSCALE` (Work-only): scale the selected Text/MText glyph height and wrap
    /// width. Full contract: [`crate::command_id::text_batch::TextScaleCommand`].
    TextScale,
    /// `TEXTALIGN` (Work-only): move the selected Text/MText to one common edge
    /// or centre. Full contract:
    /// [`crate::command_id::text_batch::TextAlignCommand`].
    TextAlignEdges,
    /// `TCOUNT` (Work-only): replace the selected text with a numbered run. Full
    /// contract: [`crate::command_id::text_batch::TCountCommand`].
    TextCount,
    /// `TCIRCLE` (Work-only): draw an estimated-bounds boundary around the
    /// selected text. Full contract:
    /// [`crate::command_id::text_batch::TCircleCommand`].
    TextCircle,
    /// `TEXTMASK` (Work-only): insert an independent `AcDbWipeout` mask behind
    /// each selected text. Full contract:
    /// [`crate::command_id::text_batch::TextMaskCommand`].
    TextMask,
    /// `TEXTUNMASK` (Work-only): delete selected masks covered by selected text.
    /// Full contract: [`crate::command_id::text_batch::TextUnmaskCommand`].
    TextUnmask,
    /// The 2026-11-07 selection/filter batch (`QSELECT`, `FILTER`, `GETSEL`,
    /// `SSX`, `SELECTCOUNT`, `-FIND`, `FIND`, `HIDEOBJECTS`,
    /// `ISOLATEOBJECTS`, `UNISOLATEOBJECTS`). Full contracts:
    /// [`crate::command_id::select_filter`].
    QSelect,
    FilterSelect,
    GetSelection,
    SelectMatchingProperties,
    SelectCount,
    FindReport,
    FindReplace,
    HideObjects,
    IsolateObjects,
    UnisolateObjects,
    /// `REDRAW`: refresh the current viewport's display without moving the
    /// camera. View-only: no database write, revision bump, undo entry or
    /// `ZOOM PREVIOUS` step. Payload-free; a caller payload is refused.
    Redraw,
    /// `REDRAWALL`: the same refresh for every viewport of the open document.
    RedrawAll,
    /// `REGEN`: regenerate the current viewport's derived representation by
    /// advancing its regeneration stamp. View-only (no database write).
    Regen,
    /// `REGENALL`: the same regeneration for every viewport of the document.
    RegenAll,
    /// `VIEW`: the session-only named-view store. Typed
    /// `SAVE <name>` / `RESTORE <name>` / `LIST` / `DELETE <name>`, bounded to
    /// 32 case-insensitively unique names. `-VIEW` is an alias of the same path.
    View,
    /// `VIEWGO <name>`: restore a named view (the same restore path as
    /// `VIEW RESTORE`, never a second view authority).
    ViewGo,
    /// `VIEWBACK`: restore the previous discrete view from the per-viewport
    /// `ZOOM PREVIOUS` history; an empty history is a named no-op diagnostic.
    ViewBack,
    /// `VIEWFORWARD`: move forward through the bounded forward stack recorded
    /// when a back step pops; an empty forward stack is a named no-op.
    ViewForward,
    /// `VPOINT <x> <y> <z>` or `TOP|BOTTOM|FRONT|BACK|LEFT|RIGHT`: set the
    /// camera viewing direction. A zero or non-finite vector is refused; the
    /// command never changes the model.
    VPoint,
}

mod capabilities;
mod mode;
pub use mode::AppMode;

pub mod area_object;
pub mod block_define;
pub mod boundary;
pub mod break_at_point;
pub mod construction_line;
pub mod dimension_chain;
pub mod dimension_extra;
pub mod draw_variants;
pub mod gradient;
pub mod grip_stretch;
pub mod hatch;
pub mod hatch_generate_boundary;
pub mod hatch_to_back;
pub mod hatchedit;
pub mod layers;
pub mod massprop;
pub mod match_prop;
pub mod measuregeom;
pub mod oops;
pub mod overkill;
pub mod pedit;
pub mod point_style;
pub mod reverse;
pub mod select_filter;
pub mod text_batch;
pub mod text_commands;
pub mod texttofront;
pub mod zoom;
