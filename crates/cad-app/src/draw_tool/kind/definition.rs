//! The [`DrawToolKind`] variant catalogue (one variant per captured operation).
//!
//! Split out of `kind.rs` to keep that module inside the code-size limit. The kind
//! is re-exported by the parent under its original name, so
//! `crate::draw_tool::DrawToolKind` and the crate-root re-export are unchanged.

/// The drawing/editing operation a tool captures for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawToolKind {
    /// LINE: two points (start, end).
    Line,
    /// CIRCLE: centre plus a point on the circumference (the radius point).
    Circle,
    /// ARC: three points (start, a point on the arc, end).
    Arc,
    /// PLINE: an open-ended polyline (at least two points, then Enter).
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now, so the honest union of
    /// supported options stays explicit):
    /// * no arc segments / bulge authoring — every captured vertex joins its
    ///   neighbour with a straight segment (`bulges = [0.0; n]`); the `Arc`
    ///   sub-command is not offered;
    /// * no `Close` — the created polyline is always `closed = false`;
    /// * no `Width` / `Halfwidth` — no per-segment start/end width is authored;
    /// * no `Undo` at a vertex prompt — the capture is all-or-nothing (cancel
    ///   discards it, Enter confirms it), and there is no vertex-level undo;
    /// * no 3D / OCS input — like LINE/CIRCLE/ARC this is the flat work-plane
    ///   subset, so points carry only the `+Z` the picker produced and are never
    ///   silently reprojected onto another plane.
    Polyline,
    /// RECTANG: two opposite corners of an axis-aligned rectangle.
    ///
    /// The commit is a **closed** four-vertex polyline (`closed = true`, one zero
    /// bulge per vertex), which is the LWPOLYLINE the database already validates
    /// and the lossy DXF export already maps exactly.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now, so the honest union of
    /// supported options stays explicit):
    /// * no `Chamfer` / `Fillet` — the four corners are always square, and the
    ///   AutoCAD quirk of a fillet/chamfer radius persisting across RECTANG
    ///   invocations within a session is not modelled (there is no tool-parameter
    ///   channel yet, see ADR 0015 decision C);
    /// * no `Thickness` / `Elevation` — the rectangle is a flat `+Z` planar
    ///   construction; no extrusion thickness and no work-plane elevation offset;
    /// * no `Width` — no polyline start/end width is authored (DXF default 0);
    /// * no `Area` / `Dimensions` — the two numeric area/dimension entry modes are
    ///   not offered (they would need the numeric input channel);
    /// * no `Rotation` — the rectangle is axis-aligned in the world frame, never
    ///   rotated by a typed angle;
    /// * no 3D / OCS input — like LINE/CIRCLE/ARC this is the flat work-plane
    ///   subset, so a pair of corners that is not coplanar on one `+Z` plane is an
    ///   explicit refusal rather than a silent projection.
    Rectang,
    /// ELLIPSE: three points by AutoCAD's **axis-endpoint method** (the two
    /// endpoints of the first axis, then a third point giving the distance to the
    /// other axis), committed as one `SemanticGeometry::Ellipse`.
    ///
    /// The centre method (`C`) is **NOT IMPLEMENTED**; this is the axis-endpoint
    /// default only. The three picks are captured exactly as clicked; the
    /// geometry-side construction, the axis-swap rule and every degeneracy
    /// refusal live in the command layer (`ellipse_parameters`), so the tool never
    /// invents a centre, a ratio or an axis.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now, so the honest union of
    /// supported options stays explicit):
    /// * no `Center` — the centre construction is not offered;
    /// * no `Arc` — only a full ellipse (`start = 0`, `sweep = 2π`) is authored;
    /// * no `Rotation` / `Isocircle` / `PELLIPSE` — no rotation parameter, no
    ///   isometric circle and no legacy polyline ellipse;
    /// * no 3D / OCS input — like LINE/CIRCLE/ARC this is the flat work-plane
    ///   subset, so a non-coplanar (`+Z`) triple is an explicit refusal rather
    ///   than a silent projection.
    Ellipse,
    /// POLYGON: the side count (a session sysvar, ADR 0015 decision C) plus the
    /// centre and a radius point, committed as one **closed** LWPOLYLINE whose
    /// `n` vertices are **inscribed** in the circle through the two picks.
    ///
    /// The two captured points are the centre (pick 1) and the radius point
    /// (pick 2); the geometry-side construction ([`polygon_vertices`]) and every
    /// degeneracy refusal live in the pure helper, so the tool never invents a
    /// vertex or a rotation.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now, so the honest union of
    /// supported options stays explicit):
    /// * no `Edge` branch — only the centre+radius construction is offered;
    /// * no **Circumscribed** (tangent) mode — the polygon is always inscribed
    ///   (AutoCAD's default), never silently circumscribed;
    /// * no numeric-radius orientation — a typed radius would rotate the first
    ///   edge to the snap rotation angle, which is not modelled; the pointer's
    ///   direction sets the rotation;
    /// * no persisted side count — the count is session/display state only;
    /// * no 3D / OCS input — like LINE/CIRCLE/ARC this is the flat work-plane
    ///   subset, so a radius point off the centre's `+Z` plane is an explicit
    ///   refusal rather than a silent projection.
    Polygon,
    /// POINT: a single placement of one point at the clicked position.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): the `PDMODE`/`PDSIZE` point
    /// styles are not modelled (the overlay draws a fixed cross marker), and
    /// AutoCAD's repeat-until-Enter placement is **not** implemented — the ADR
    /// records that the POINT page documents only a single `Specify a point` and
    /// that repetition is unverified, so it is not guessed. The point carries the
    /// `+Z` the picker produced and is never silently reprojected.
    Point,
    /// SPLINE: a fit-point spline (at least four fit points, then Enter), stored
    /// as a degree-3 clamped chord interpolating B-spline.
    ///
    /// The interpolation itself lives in the pure geometry core
    /// ([`cad_geometry::spline::fit_points_to_cubic_spline`]); the tool only
    /// captures the ordered fit points. Deliberate subset (NOT IMPLEMENTED for
    /// now): the `CV` method, the `Knots`/`Degree`/`Start`/`End Tangency`/
    /// `Tolerance` options, `Close` and `Object` conversion; the 2–3 fit-point
    /// small-sample inputs (unverified) are explicitly refused, and fit-point data
    /// does not round-trip (the domain stores only knots/control points/weights).
    Spline,
    /// MOVE: requires a selection, then a two-point delta.
    Move,
    /// TRIM: a target pick plus a boundary pick.
    Trim,
    /// COPY: requires a selection, then a two-point delta (duplicates).
    Copy,
    /// EXTEND: a target pick plus a boundary pick (grows the target LINE).
    Extend,
    /// ROTATE: requires a selection, then a base point and a direction point.
    Rotate,
    /// SCALE: requires a selection, then a base, reference and target point.
    Scale,
    /// MIRROR: requires a selection, then the two points of the mirror axis.
    Mirror,
    /// OFFSET: requires a single-entity selection, then one side/distance point.
    Offset,
    /// `COPYBASE`: requires a selection, then one **base point** (AutoCAD's
    /// "Specify base point").
    ///
    /// The capture is read-only: the command clones the selection snapshot taken at
    /// tool start into the in-process clipboard and records the captured base, so a
    /// later `PASTECLIP` can translate by `insertion − base`. No point is captured
    /// beyond the base, and no transaction is opened (Viewer mode may run it).
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): the OS/system clipboard is
    /// never touched (the clipboard is session state, cleared on document
    /// replacement), there is no `Duplicate`/`Multiple` paste mode and no
    /// scaled/rotated paste.
    CopyBase,
    /// `CUTBASE`: the [`Self::CopyBase`] capture for the **cutting** command
    /// (`CUTBASE`), requires a selection, then one base point.
    ///
    /// Why a second kind instead of one shared kind: the typed
    /// [`crate::interaction::InteractionRequest::BeginDraw`] request carries a
    /// [`DrawToolKind`] and **no** [`crate::CommandId`], and the two commands could
    /// not differ anywhere else — `CUTBASE` is work-only while `COPYBASE` is
    /// Viewer-legal, and the confirmed payload (`CutBase` vs `CopyBase`, i.e.
    /// delete or not) differs. The kind is therefore the transport for that one
    /// bit, and the session records the matching `CommandId` in the capture so the
    /// host maps the payload by the command (never by guessing). The captured
    /// point is identical, so both kinds produce
    /// [`crate::DrawIntent::CopyBase`].
    ///
    /// The delete happens in the command layer, after the capture, in one
    /// transaction and one undo step.
    CutBase,
    /// `PASTECLIP`: one **insertion point**, no selection requirement.
    ///
    /// The clipboard is the input, so this kind does not require a selection; an
    /// empty clipboard is refused by the command (never an empty success). The
    /// captured point is the insertion point, and the command translates every
    /// clipboard entry by `insertion − clipboard base` in one transaction / one
    /// undo step. Work-only through [`crate::CommandId::PasteFromClipboard`].
    PasteClip,
    /// CHAMFER: two straight-line entity picks (ADR 0016 decision C).
    ///
    /// The two clicks are **entity picks** (the TRIM/EXTEND capture shape), not
    /// free points: the first-picked line is "the first object" and its raw click
    /// point selects the pick side; the two distances are the session typed
    /// channel (`ChamferDistanceA`/`B`). The geometry construction and every
    /// refusal live in the command layer.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): the `Angle` method and
    /// `CHAMMODE`, the `Polyline` mode, `Multiple`, in-command `Undo`, the
    /// `Trim`/`No Trim` switch (the lines are always trimmed, `TRIMMODE=1`), the
    /// "replace the intermediate arc" case, and 3D.
    Chamfer,
    /// FILLET: two straight-line entity picks (ADR 0016 decision B).
    ///
    /// Same capture shape as [`Self::Chamfer`]; the radius is the session typed
    /// channel (`FilletRadius`, default 0 ⇒ corner only). The tangent-arc centre
    /// construction is a labelled geometric inference (Autodesk publishes no
    /// formula); the command layer builds it and trims both lines to the tangent
    /// points.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): the `Polyline` mode,
    /// `Multiple`, in-command `Undo`, the `Trim`/`No Trim` switch, the automatic
    /// parallel-line radius adjustment (we refuse parallel lines explicitly),
    /// Circle/Arc targets, 3D, and hatch associativity.
    Fillet,
    /// ARRAYRECT: insert independent copies of the selection on a rectangular
    /// grid (ADR 0016 decision D).
    ///
    /// This kind captures **no points**: the array is defined entirely by the
    /// session typed parameters ([`super::ToolParameter::ArrayRows`],
    /// `ArrayColumns`, `ArrayRowSpacing`, `ArrayColumnSpacing`) plus the selection
    /// snapshot taken at tool start. It is therefore ready as soon as the
    /// selection is non-empty — its arity is the zero-point complement
    /// ([`PointArity::Exact`]`(0)`), and the state machine never reads a point for
    /// it.
    ///
    /// Our declared geometry rule (AutoCAD's exact base-point rule is unverified):
    /// the base point is the selection's **bounding-box lower-left**, cell
    /// `(row, column)` translates the source by `(column·columnSpacing,
    /// row·rowSpacing)`, and the array grows **up and to the right**. Spacing is
    /// the distance between base points.
    ///
    /// `rows × columns` is the **total including the source object**, which stays
    /// where it is and is not duplicated, so the array inserts `rows × columns − 1`
    /// independent copies; the source's own cell `(0, 0)` is skipped and a 1×1
    /// array produces **0 copies** (an explicit diagnostic, never a silent
    /// success). The source objects are **retained** (AutoCAD's `DELOBJ` gating is
    /// not modelled).
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): associativity /
    /// `ARRAYEDIT` / grips, `Path` arrays, `Levels`, `Total` and angle-interval
    /// inputs, a configurable `MAXARRAY`, 3D, and the negative-spacing reversal
    /// (a negative spacing is refused rather than inverted).
    ArrayRect,
    /// ARRAYPOLAR: insert independent copies of the selection around a captured
    /// centre (ADR 0016 decision D).
    ///
    /// This kind captures exactly **one point** (the array centre); the item
    /// count, fill angle and rotate-items flag are the session typed parameters
    /// ([`super::ToolParameter::ArrayItems`], `ArrayFillAngle`,
    /// `ArrayRotateItems`).
    ///
    /// Our declared, **unverified** placement rule (the command layer builds it and
    /// the contract tests pin it): item `i` (`i` in `0..items`) is placed at angle
    /// `i·fillAngle / items`, measured counter-clockwise from the base point's
    /// direction. With `rotateItems = true` each copy is **rotated about the
    /// centre** by that angle; with `false` each copy is **translated only** to its
    /// angular position (its own orientation unchanged), using the selection's
    /// bounding-box lower-left as the base point.
    ///
    /// `items` is the **total including the source object**, which sits at its own
    /// direction and is not duplicated, so the array inserts `items − 1`
    /// independent copies; the item whose angle offset is 0 (item 0) is skipped and
    /// a 1-item array produces **0 copies** (an explicit diagnostic, never a silent
    /// success). The source objects are **retained**.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): associativity /
    /// `ARRAYEDIT` / grips, `Path` arrays, `Levels`, `Total` and angle-interval
    /// inputs, a configurable `MAXARRAY`, and 3D.
    ArrayPolar,
    /// ALIGN: two pairs of points (`Exact(4)`) that move+rotate the selection
    /// (ADR 0016 decision F).
    ///
    /// Capture order follows AutoCAD: the first **source** point, its
    /// **destination**, then the second source point and its destination. The
    /// transform translates `source1 → dest1`, rotates by the angle between the
    /// two source/destination directions, and (only when the session
    /// [`super::ToolParameter::AlignScale`] flag is true) applies a **uniform**
    /// scale of `|dest2 − dest1| / |source2 − source1|` about `source1`. The
    /// command layer builds and applies the composed transform to the whole
    /// selection **in place** (no copies) in one transaction / one undo step; the
    /// flag is the session typed channel, read by the command layer — never a
    /// payload field.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): three-pair 3D alignment,
    /// **non-uniform** scaling, and the one-pair "move only" case (its effect is
    /// exactly MOVE, and the `PointArity` model cannot express AutoCAD's
    /// even-point "2 or 4 points" prompt — see ADR 0016 decision F).
    Align,
    /// STRETCH: the two opposite corners of a **crossing window**, then a base
    /// point and a displacement point (`Exact(4)`, no entity picks; ADR 0016
    /// decision E).
    ///
    /// The window itself defines the affected set, so this kind
    /// [`DrawToolKind::requires_selection`] is **false**. The window is always
    /// treated as **CROSSING** (AutoCAD requires a crossing window; cpolygon and
    /// lasso selection are NOT IMPLEMENTED). The enclosure classification
    /// (fully enclosed ⇒ whole-entity move; partially enclosed ⇒ per-vertex
    /// stretch) is recomputed by the command layer from the rectangle and the
    /// geometry, never stored here.
    ///
    /// The two window corners are captured as world points (the same free-point
    /// capture every drawing tool uses); the host maps them back to canvas
    /// logical corners for the payload's `rect_logical`, which the command maps
    /// through the orthographic plan view exactly like `BoxSelect`.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): cpolygon/lasso selection,
    /// the "select an object directly ⇒ move it whole" clause, stretching of
    /// `TEXT`/`SPLINE`/`ARC`/`HATCH`/`INSERT` (TEXT fails the whole command;
    /// the others are move-only when fully enclosed and unchanged when
    /// partially enclosed), 3D, and width/tangency edits (polyline width is not
    /// modelled). The bulge-preservation rule for a dragged bulge-segment
    /// endpoint is **UNVERIFIED** (Autodesk does not document it) and is our
    /// declared choice.
    Stretch,
    /// DIMLINEAR: a horizontal/vertical linear dimension through two
    /// extension-line origins plus the dimension-line location (`Exact(3)`, ADR
    /// 0017 decision C, P5c).
    ///
    /// Capture order follows AutoCAD: the first extension-line origin, the second
    /// origin, then the dimension-line location. The **location pick decides
    /// horizontal vs vertical** (AutoCAD's rule — the location determines the
    /// direction): if the location is offset more vertically than horizontally
    /// from the origins' midpoint the dimension is horizontal (measures X), else
    /// vertical (measures Y). The command layer builds the `DimensionGeometry`
    /// through the shared synthesis and owns every refusal (non-plan triple,
    /// coincident origins, ambiguous direction, zero measurement).
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): the **object-selection**
    /// variant (pick a line/circle/arc to derive the origins), a `Text`/`Mtext`
    /// text override (the automatic measurement is always drawn),
    /// associativity (`DIMASSOC=2`), and the object-selection prompts. The
    /// location→direction rule and the exact prompt state machine are our
    /// declared reading (the strict AutoCAD prompt sequence is unverified).
    DimLinear,
    /// DIMALIGNED: a dimension whose line is parallel to the line joining the two
    /// extension-line origins (`Exact(3)`, ADR 0017 decision C, P5c).
    ///
    /// Same three-pick capture as [`Self::DimLinear`] (two origins then the
    /// dimension-line location); the measurement is the true origin-to-origin
    /// distance and the dimension line runs parallel to that segment. The command
    /// layer owns every refusal (non-plan triple, coincident origins, zero
    /// measurement).
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): the **object-selection**
    /// variant, a text override, associativity (`DIMASSOC=2`), and the
    /// object-selection prompts.
    DimAligned,
    /// DIMRADIUS: a radius dimension whose **only** entry point is an entity
    /// pick of an arc or circle, followed by the dimension-line location
    /// (`Exact(2)`, ADR 0017 decision C, P5d).
    ///
    /// Autodesk documents "Select arc or circle" as the sole entry point, so —
    /// unlike DIMLINEAR/DIMALIGNED — this kind **starts with an entity pick**
    /// ([`Self::picked_step_count`] is `1`): the first step resolves an existing
    /// arc/circle ref through the same pick path TRIM/EXTEND use, and the second
    /// is a free point giving the leader/dimension-line direction. A **fillet
    /// counts**: FILLET commits an `Arc`, so a filleted corner is a legal pick.
    ///
    /// The location pick determines the **direction** from the circle centre; the
    /// leader's point on the arc is the centre plus the radius along that
    /// direction. The command layer builds the definition (group 15 = that point,
    /// group 10 = the centre for a radius), the measurement (the radius) and every
    /// refusal (a non-arc/circle pick, an off-plane entity or location, a
    /// degenerate radius, a location at the centre, a missing active layer).
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): the object-selection variant
    /// is **not** applicable (this command *is* the object pick), a `Text`/`Mtext`
    /// override, associativity (`DIMASSOC=2`), and clamping the leader point to an
    /// **arc's own sweep** (the point lies on the arc's circle in the picked
    /// direction; a direction outside the sweep is not clamped — our declared
    /// reading).
    DimRadius,
    /// DIMDIAMETER: a diameter dimension whose only entry point is an entity pick
    /// of an arc or circle, followed by the dimension-line location (`Exact(2)`,
    /// ADR 0017 decision C, P5d).
    ///
    /// Same two-step capture as [`Self::DimRadius`] (an entity pick then a free
    /// location point). The definition's group-10 slot is the **opposite chord
    /// point** (centre minus the radius along the picked direction) rather than
    /// the centre, and the measurement is the **diameter** (ADR 0017 A.3). Every
    /// refusal matches DIMRADIUS.
    DimDiameter,
    /// DIMCENTER: create the **non-associative centre mark or centrelines** of a
    /// picked arc/circle as **plain geometry** (`Exact(1)`, ADR 0017 decision C,
    /// P5d).
    ///
    /// Autodesk documents DIMCENTER as creating a "non-associative center mark or
    /// centerlines", i.e. ordinary LINE geometry — **not** a `DIMENSION` entity
    /// (ADR 0017 decision D.7). The one step is an entity pick of an arc/circle.
    /// The `DIMCEN` value drives the result (pinned sign reading): `> 0` a centre
    /// mark (a crossed pair of LINEs), `< 0` centrelines (a crossed pair reaching
    /// past the circle), `0` **nothing** — an explicit diagnostic with **no
    /// transaction**, never a silent empty success. The command layer owns every
    /// refusal (a non-arc/circle pick, an off-plane entity, a degenerate radius, a
    /// missing active layer).
    ///
    /// Declared approximation: AutoCAD's negative-`DIMCEN` centrelines are a centre
    /// mark plus **gapped** centrelines; we draw a plain cross, the same declared
    /// approximation as the radial synthesis (ADR 0017 P5b deviation 3).
    DimCenter,
    /// DIMANGULAR (two-line form): two straight-line entity picks followed by the
    /// dimension-arc location (`Exact(3)`, ADR 0017 decision C, P5e).
    ///
    /// The first two steps are **entity picks** (the TRIM/EXTEND capture shape,
    /// [`Self::picked_step_count`] is `2`); the last is a free point. The vertex is
    /// the intersection of the two infinite lines and the location pick selects
    /// which of the four angles is dimensioned (ADR 0017 U-1: our declared rule is
    /// "the sector containing the location pick"; U-8 is pinned by the committed
    /// QCAD fixtures). The command layer owns every refusal (non-line pick,
    /// parallel/collinear lines, coincident points, off-plane input, a location at
    /// the vertex).
    ///
    /// Declared subset: only the **two-line** construction is captured here. The
    /// **three-point** construction (vertex + two side points + arc location) is
    /// supported by the command payload but has no interactive capture yet — the
    /// fixed-step state machine cannot express AutoCAD's "select a line **or**
    /// specify a vertex" first step. Arc/circle objects and angular dimensions
    /// derived from them are NOT IMPLEMENTED.
    DimAngular,
    /// DIMORDINATE: a feature point and a leader endpoint (`Exact(2)`, ADR 0017
    /// decision C, P5e).
    ///
    /// Both steps are free points (no entity pick). The command layer derives the
    /// axis from the documented auto rule (a greater Y difference measures the X
    /// ordinate; the unstated equal case is our declared choice of Y), unless a
    /// payload override forces one. The text override prompt and the object
    /// selection variant are NOT IMPLEMENTED.
    DimOrdinate,
    /// DIMBASELINE: pick **one existing linear dimension** as the base, then N
    /// points; Enter confirms N stacked dimensions (ADR 0017 decision C, P5f).
    ///
    /// The base is an **explicit pick inside this invocation**
    /// ([`Self::picked_step_count`] is `1`), never a remembered "last dimension",
    /// so the tool holds no cross-command state and undo/erase/document switches
    /// cannot leave a dangling base. The base pick is stored as a capture point
    /// too, so the arity is [`PointArity::AtLeast`]`(2)` — the pick **plus at
    /// least one measured point**; a base pick alone is never confirmable. More
    /// points are accepted up to the shared cap and Enter finishes the capture.
    ///
    /// The row construction (base rotation, `first = base.first`,
    /// `dimension_line = base.location + n̂ · (i · DIMDLI)`), the normal selection
    /// and every refusal live in the command layer
    /// (`crate::app_drawing::dimension_chain`), so the capture never invents a
    /// stacking direction.
    ///
    /// Deliberate subset (NOT IMPLEMENTED): bases other than a linear dimension
    /// (aligned/angular/radial/ordinate are named refusals), the repeat-until-Esc
    /// loop, per-point undo, associativity and a text override.
    DimBaseline,
    /// DIMCONTINUE: pick one existing linear dimension as the base, then N points
    /// forming a chain from the base's second origin (ADR 0017 decision C, P5f).
    ///
    /// Same capture shape as [`Self::DimBaseline`] (one leading entity pick, then
    /// an open-ended point run). The chain geometry — segment 1 `base.second →
    /// P_1`, segment `i` `P_{i−1} → P_i`, all on `base.location`, no stacking, and
    /// the named refusal of a segment that does not advance — lives in the command
    /// layer.
    DimContinue,
    /// HATCH: fill the **pre-selection** (ADR 0018 decision D, P6b).
    ///
    /// This kind captures **no points** — its arity is the zero-point exact
    /// complement, exactly like [`Self::ArrayRect`] — and takes **no in-command
    /// entity picks** ([`Self::picked_step_count`] is `0`). It is defined by the
    /// selection snapshot taken at tool start (each selected entity becomes one
    /// boundary loop) plus the session typed parameters
    /// ([`super::ToolParameter::HatchPatternType`], `HatchPattern`, `HatchSpacing`,
    /// `HatchAngle`, `HatchDouble`, `HatchStyle`), so it is ready as soon as the
    /// selection is non-empty.
    ///
    /// Accepted boundaries are `Circle`, a full `Ellipse`, a **closed** `Polyline`
    /// (bulges handled) and `LINE`/`ARC` edges, which are **stitched** into one
    /// loop per closed chain at the absolute `1e-6` world-unit tolerance with
    /// every arc kept as an exact bulge segment (`bulge = tan(sweep / 4)`, never
    /// sampled). Every other kind is an explicit named refusal that opens no
    /// transaction; so are a stitched branch node (`hatch.boundary_branch`), an
    /// unconsumed open chain (`hatch.boundary_open`) and a degenerate
    /// `LINE`/`ARC` (`hatch.boundary_degenerate`). The command layer owns the
    /// plane/normal consistency check, the loop construction, the pattern
    /// synthesis and every `HatchGeometry::new` creation-path rejection
    /// (self-intersecting loops included — they are refused, not repaired).
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): boundary from an internal
    /// point, drawn boundaries, `remove boundaries`, custom (`76=2`) pattern
    /// names and `.pat` loading (no parser and no packaged pattern library; a
    /// pattern picker dialog is also missing — only the three transcribed
    /// predefined entries `ANSI31`/`ANSI32`/`ANSI37` are selectable), gradient
    /// creation, associativity (`HPASSOC`), spline boundaries, island detection
    /// beyond the style semantics, a per-span dash phase reset, and a live hatch
    /// preview (the React host cannot paint an overlay above the native CAD
    /// partition).
    Hatch,
    /// TEXT: one insertion point, then the text itself (ADR 0019 decision A/F.1).
    ///
    /// This is the **first** tool whose capture is not defined by points alone:
    /// the kind [`Self::requires_text`] and its last step is a typed string
    /// submitted through the dedicated `cad_set_tool_text` channel
    /// ([`crate::interaction::InteractionRequest::SetToolText`]). That request is
    /// **terminal**: once the point complement is complete, the submitted text
    /// completes the command immediately (no second `Confirm`).
    ///
    /// The height and the rotation are the session typed parameters
    /// [`super::ToolParameter::TextHeight`] (finite, `> 0`, default `2.5` — the
    /// documented constant, because the drawing's `Style` table carries no
    /// height) and [`super::ToolParameter::TextRotation`] (any finite degrees
    /// value, default `0`, converted once to the stored radians), so the capture
    /// never invents one.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): a per-invocation height /
    /// rotation prompt (both are session parameters with no in-tool prompt yet),
    /// the `Justify` / `Style` / `Align` / `Fit` / `Center` /
    /// `Middle` / `Right` / `TL` sub-prompts (the stored alignment is always the
    /// style default, left/baseline), width factor, oblique angle, character
    /// spacing, background/mask and any in-canvas in-place editor (the React
    /// host cannot draw a DOM editor over the native CAD partition — ADR 0002).
    /// MTEXT's columns/background/line-spacing style/drawing direction are
    /// likewise not modelled (ADR 0019 decision H).
    Text,
    /// MTEXT: the two opposite corners of the wrap rectangle, then the text
    /// (interactive subset, 2026-10-26).
    ///
    /// Capture order: the **first corner** is the insertion point (`position`);
    /// the **opposite corner** only supplies the wrap `width` (`|Δx|`, finite and
    /// `> 0`, derived by the host — a degenerate rectangle is an explicit
    /// `tool.mtext_zero_rect` refusal, never a fabricated width). The glyph
    /// `height` is the session typed channel
    /// [`super::ToolParameter::TextHeight`] (the same `2.5` default TEXT uses),
    /// the `rotation` is [`super::ToolParameter::TextRotation`] (degrees, default
    /// `0`, converted once to the stored radians) and the attachment is
    /// [`super::ToolParameter::MTextAttachment`] (AutoCAD's `1..=9` codes, default
    /// `1` = top-left → `TextAlignH::Left` + `TextAlignV::Top`). The text step is
    /// the same terminal typed channel as [`Self::Text`]; newlines are allowed and
    /// stored as `\P` by the shared policy (`AcDbMText`).
    ///
    /// Deliberate subset (NOT IMPLEMENTED, so the honest union stays explicit):
    /// an in-tool rotation / attachment prompt (both are session parameters with
    /// no prompt yet; the GUI comes later), user-settable justification outside the
    /// nine attachment codes, line spacing, columns, background fill, drawing
    /// direction and any in-command MTEXT editing dialog. None of them has a
    /// prompt or a payload field.
    MText,
    /// MLEADER: the arrow position, then the landing point, then the text
    /// (ADR 0019 decision F.2, P7c).
    ///
    /// Two free points ([`PointArity::Exact(2)`]) and no entity picks
    /// ([`Self::picked_step_count`] is `0`), followed by the **same** terminal
    /// typed-text step [`Self::Text`] uses ([`Self::requires_text`]); submitting
    /// the text completes the command. The commit is one root, one line and
    /// **two vertices** with a straight path — the dogleg start is derived from
    /// the landing point and the style's dogleg length by the command layer
    /// (`MultiLeaderGeometry::created`), never captured.
    ///
    /// Every style value (dogleg length, landing gap, arrowhead size, text
    /// height, text style, vertical attachment) comes from the drawing's
    /// `MLeaderStyle` table. Text is **required**: content-less MULTILEADER is
    /// import/display/export only, so an empty text is refused and `Confirm`
    /// while the text prompt is open is refused by name.
    ///
    /// Deliberate subset (NOT IMPLEMENTED): **no `Options` sub-prompt** — no
    /// leader type (straight/spline), no content type (block/tolerance), no
    /// maximum-points/angular-constraint option and no per-invocation dogleg or
    /// attachment prompt; the UI must not imply them. Multi-column text,
    /// background masks, per-line overrides, associativity and editing an
    /// existing leader are likewise not offered.
    MLeader,
    /// TEXTEDIT: pick **one existing single-line `Text`** entity, then replace its
    /// string (ADR 0020 amendment options A+B).
    ///
    /// The first step is an **entity pick** ([`Self::picked_step_count`] is `1`,
    /// exactly like [`Self::DimRadius`]); the host resolves the pick through the
    /// normal pick path and refuses anything that is not a single, directly
    /// addressable `Text` entity (a line, a hatch, a dimension, a compound, a
    /// block-instance member) **at capture time**, so the step is never consumed
    /// by a wrong object. When the pick resolves, the host fills
    /// [`DrawPreview::text_prefill`] with the entity's **current** string so the UI
    /// can show it for editing.
    ///
    /// The second and terminal step is the same typed-text channel as [`Self::Text`]
    /// ([`Self::requires_text`], `SetToolText`). On submit the host re-reads the
    /// picked entity inside the command and replaces only its text; if the entity
    /// was deleted or replaced since the pick, the command refuses by name and
    /// opens no transaction.
    ///
    /// Deliberate subset (NOT IMPLEMENTED): whole-string replacement only — there
    /// is no cursor-level/inline editor, no MTEXT markup parsing or re-flow, and
    /// no editing of MLEADER/compound children or block-internal members (they
    /// have no addressable identity; ADR 0020 postpones MLEADER editing to P7b).
    TextEdit,
}
