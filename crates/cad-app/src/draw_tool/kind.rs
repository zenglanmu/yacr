//! The drawing/editing kind catalogue: kinds, arities, keys, labels and the
//! per-kind metadata the capture state machine reads.
//!
//! Split out of [`super`] to keep the state-machine module inside the code-size
//! limit. Every item here is re-exported by [`super`] under its original name,
//! so `crate::draw_tool::DrawToolKind`, `crate::draw_tool::PointArity` and the
//! crate-root re-exports are unchanged.

/// How many points a capture needs: a fixed complement, or an open-ended
/// minimum.
///
/// The interactive tools were historically fixed-arity (`required_points`),
/// which cannot express a polyline: PLINE accepts as many vertices as the user
/// clicks and is finished with Enter. Making the arity explicit lets the state
/// machine keep the two models apart instead of pretending an open-ended tool
/// has a fixed "0 remaining".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointArity {
    /// Exactly this many points; the `(n + 1)`-th is refused.
    Exact(usize),
    /// At least this many points; further points are accepted (up to the hard
    /// cap [`MAX_OPEN_CAPTURE_POINTS`]) and the capture is confirmed with Enter.
    AtLeast(usize),
}

impl PointArity {
    /// The minimum number of points required to define the operation.
    pub fn minimum(self) -> usize {
        match self {
            Self::Exact(n) | Self::AtLeast(n) => n,
        }
    }

    /// Whether the capture accepts points beyond the minimum (open-ended).
    pub fn is_open_ended(self) -> bool {
        matches!(self, Self::AtLeast(_))
    }
}

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
    /// HATCH: fill the **pre-selection** (ADR 0018 decision D, P6b).
    ///
    /// This kind captures **no points** — its arity is the zero-point exact
    /// complement, exactly like [`Self::ArrayRect`] — and takes **no in-command
    /// entity picks** ([`Self::picked_step_count`] is `0`). It is defined by the
    /// selection snapshot taken at tool start (each selected entity becomes one
    /// boundary loop) plus the session typed parameters
    /// ([`super::ToolParameter::HatchPatternType`], `HatchSpacing`, `HatchAngle`,
    /// `HatchDouble`, `HatchStyle`), so it is ready as soon as the selection is
    /// non-empty.
    ///
    /// Accepted boundaries are `Circle`, a full `Ellipse` and a **closed**
    /// `Polyline` (bulges handled); every other kind is an explicit named refusal
    /// that opens no transaction. The command layer owns the plane/normal
    /// consistency check, the loop construction, the pattern synthesis and every
    /// `HatchGeometry::new` creation-path rejection.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): boundary from an internal
    /// point, drawn boundaries, `remove boundaries`, predefined/custom pattern
    /// names (no pattern table and no `.pat` parser — the refusal suggests
    /// spacing/angle), gradient creation, associativity (`HPASSOC`), LINE/ARC
    /// boundary stitching, spline boundaries, island detection beyond the style
    /// semantics, and a live hatch preview (the React host cannot paint an
    /// overlay above the native CAD partition).
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
    /// The height is the session typed parameter
    /// [`super::ToolParameter::TextHeight`] (finite, `> 0`, default `2.5` — the
    /// documented constant, because the drawing's `Style` table carries no
    /// height), so the capture never invents one.
    ///
    /// Deliberate subset (NOT IMPLEMENTED for now): a per-invocation height /
    /// rotation prompt, the `Justify` / `Style` / `Align` / `Fit` / `Center` /
    /// `Middle` / `Right` / `TL` sub-prompts (the stored alignment is always the
    /// style default, left/baseline), width factor, oblique angle, character
    /// spacing, background/mask and any in-canvas in-place editor (the React
    /// host cannot draw a DOM editor over the native CAD partition — ADR 0002).
    /// MTEXT's columns/background/line-spacing style/drawing direction are
    /// likewise not modelled (ADR 0019 decision H).
    Text,
}

impl DrawToolKind {
    /// Every kind, in the order the UI selector presents them. This is the one
    /// authoritative ordering; the UI shell mirrors it instead of keeping a
    /// second hand-written list that can drift.
    pub const ALL: [DrawToolKind; 32] = [
        DrawToolKind::Line,
        DrawToolKind::Circle,
        DrawToolKind::Arc,
        DrawToolKind::Polyline,
        DrawToolKind::Rectang,
        DrawToolKind::Ellipse,
        DrawToolKind::Polygon,
        DrawToolKind::Point,
        DrawToolKind::Spline,
        DrawToolKind::Move,
        DrawToolKind::Trim,
        DrawToolKind::Copy,
        DrawToolKind::Extend,
        DrawToolKind::Rotate,
        DrawToolKind::Scale,
        DrawToolKind::Mirror,
        DrawToolKind::Offset,
        DrawToolKind::Chamfer,
        DrawToolKind::Fillet,
        DrawToolKind::ArrayRect,
        DrawToolKind::ArrayPolar,
        DrawToolKind::Align,
        DrawToolKind::Stretch,
        DrawToolKind::DimLinear,
        DrawToolKind::DimAligned,
        DrawToolKind::DimRadius,
        DrawToolKind::DimDiameter,
        DrawToolKind::DimCenter,
        DrawToolKind::DimAngular,
        DrawToolKind::DimOrdinate,
        DrawToolKind::Hatch,
        // Appended last so every pre-existing index stays stable (the UI mirrors
        // this order rather than keeping a second list).
        DrawToolKind::Text,
    ];

    /// Stable, locale-independent machine key (never translated).
    pub fn key(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Circle => "circle",
            Self::Arc => "arc",
            Self::Polyline => "polyline",
            Self::Rectang => "rectang",
            Self::Ellipse => "ellipse",
            Self::Polygon => "polygon",
            Self::Point => "point",
            Self::Spline => "spline",
            Self::Move => "move",
            Self::Trim => "trim",
            Self::Copy => "copy",
            Self::Extend => "extend",
            Self::Rotate => "rotate",
            Self::Scale => "scale",
            Self::Mirror => "mirror",
            Self::Offset => "offset",
            Self::Chamfer => "chamfer",
            Self::Fillet => "fillet",
            Self::ArrayRect => "arrayrect",
            Self::ArrayPolar => "arraypolar",
            Self::Align => "align",
            Self::Stretch => "stretch",
            Self::DimLinear => "dimlinear",
            Self::DimAligned => "dimaligned",
            Self::DimRadius => "dimradius",
            Self::DimDiameter => "dimdiameter",
            Self::DimCenter => "dimcenter",
            Self::DimAngular => "dimangular",
            Self::DimOrdinate => "dimordinate",
            Self::Hatch => "hatch",
            Self::Text => "text",
        }
    }

    /// Parse a machine key back to a kind; unknown keys are rejected instead of
    /// silently falling back to a default operation.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "line" => Some(Self::Line),
            "circle" => Some(Self::Circle),
            "arc" => Some(Self::Arc),
            "polyline" => Some(Self::Polyline),
            "rectang" => Some(Self::Rectang),
            "ellipse" => Some(Self::Ellipse),
            "polygon" => Some(Self::Polygon),
            "point" => Some(Self::Point),
            "spline" => Some(Self::Spline),
            "move" => Some(Self::Move),
            "trim" => Some(Self::Trim),
            "copy" => Some(Self::Copy),
            "extend" => Some(Self::Extend),
            "rotate" => Some(Self::Rotate),
            "scale" => Some(Self::Scale),
            "mirror" => Some(Self::Mirror),
            "offset" => Some(Self::Offset),
            "chamfer" => Some(Self::Chamfer),
            "fillet" => Some(Self::Fillet),
            "arrayrect" => Some(Self::ArrayRect),
            "arraypolar" => Some(Self::ArrayPolar),
            "align" => Some(Self::Align),
            "stretch" => Some(Self::Stretch),
            "dimlinear" => Some(Self::DimLinear),
            "dimaligned" => Some(Self::DimAligned),
            "dimradius" => Some(Self::DimRadius),
            "dimdiameter" => Some(Self::DimDiameter),
            "dimcenter" => Some(Self::DimCenter),
            "dimangular" => Some(Self::DimAngular),
            "dimordinate" => Some(Self::DimOrdinate),
            "hatch" => Some(Self::Hatch),
            "text" => Some(Self::Text),
            _ => None,
        }
    }

    /// Resolve a kind from its user-facing label (as displayed by the shell).
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.label() == label)
    }

    /// Position in [`Self::ALL`].
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|kind| *kind == self)
            .expect("every kind is listed in ALL")
    }

    /// Kind at a UI index, or `None` for an out-of-range index.
    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// Short user-facing label. The shell substitutes the catalog label for the
    /// same ordering, so this is a default/fallback, not the chrome source.
    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "直线",
            Self::Circle => "圆",
            Self::Arc => "圆弧",
            Self::Polyline => "多段线",
            Self::Rectang => "矩形",
            Self::Ellipse => "椭圆",
            Self::Polygon => "正多边形",
            // POINT/SPLINE deliberately use locale-neutral ASCII fallback labels:
            // the ADR fence forbids new CJK literals on the Rust side, and the
            // shell substitutes the catalog label for chrome anyway, so `label()`
            // stays a non-chrome fallback here (see the CJK baseline test).
            Self::Point => "POINT",
            Self::Spline => "SPLINE",
            Self::Move => "移动",
            Self::Trim => "修剪",
            Self::Copy => "复制",
            Self::Extend => "延伸",
            Self::Rotate => "旋转",
            Self::Scale => "缩放",
            Self::Mirror => "镜像",
            Self::Offset => "偏移",
            // CHAMFER/FILLET use locale-neutral ASCII fallback labels, exactly like
            // POINT/SPLINE: the ADR fence forbids new CJK literals on the Rust side
            // and the shell substitutes the catalog label for chrome anyway.
            Self::Chamfer => "CHAMFER",
            Self::Fillet => "FILLET",
            // ARRAYRECT/ARRAYPOLAR use locale-neutral ASCII fallback labels,
            // exactly like POINT/SPLINE/CHAMFER/FILLET: the ADR fence forbids new
            // CJK literals on the Rust side and the shell substitutes the catalog
            // label for chrome anyway.
            Self::ArrayRect => "ARRAYRECT",
            Self::ArrayPolar => "ARRAYPOLAR",
            // ALIGN uses a locale-neutral ASCII fallback label, exactly like
            // POINT/SPLINE/CHAMFER/FILLET/ARRAY*: the ADR fence forbids new CJK
            // literals on the Rust side and the shell substitutes the catalog label
            // for chrome anyway.
            Self::Align => "ALIGN",
            // STRETCH uses a locale-neutral ASCII fallback label, exactly like
            // POINT/SPLINE/CHAMFER/FILLET/ARRAY*/ALIGN: the ADR fence forbids new
            // CJK literals on the Rust side and the shell substitutes the catalog
            // label for chrome anyway.
            Self::Stretch => "STRETCH",
            // DIMLINEAR/DIMALIGNED use locale-neutral ASCII fallback labels,
            // exactly like POINT/SPLINE/CHAMFER/FILLET/ARRAY*/ALIGN/STRETCH: the
            // ADR fence forbids new CJK literals on the Rust side and the shell
            // substitutes the catalog label for chrome anyway.
            Self::DimLinear => "DIMLINEAR",
            Self::DimAligned => "DIMALIGNED",
            // DIMRADIUS/DIMDIAMETER/DIMCENTER use locale-neutral ASCII fallback
            // labels, exactly like the other post-ADR-0015 kinds: the ADR fence
            // forbids new CJK literals on the Rust side and the shell substitutes
            // the catalog label for chrome anyway.
            Self::DimRadius => "DIMRADIUS",
            Self::DimDiameter => "DIMDIAMETER",
            Self::DimCenter => "DIMCENTER",
            // DIMANGULAR/DIMORDINATE use locale-neutral ASCII fallback labels,
            // exactly like the other post-ADR-0015 kinds: the ADR fence forbids
            // new CJK literals on the Rust side and the shell substitutes the
            // catalog label for chrome anyway.
            Self::DimAngular => "DIMANGULAR",
            Self::DimOrdinate => "DIMORDINATE",
            // HATCH uses a locale-neutral ASCII fallback label, exactly like the
            // other post-ADR-0015 kinds: the ADR fence forbids new CJK literals on
            // the Rust side and the shell substitutes the catalog label for chrome
            // anyway.
            Self::Hatch => "HATCH",
            // TEXT uses a locale-neutral ASCII fallback label, exactly like the
            // other post-ADR-0015 kinds: the ADR fence forbids new CJK literals on
            // the Rust side and the shell substitutes the catalog label for chrome
            // anyway.
            Self::Text => "TEXT",
        }
    }

    /// The capture arity: an exact complement, or an open-ended minimum.
    pub fn arity(self) -> PointArity {
        match self {
            // PLINE and SPLINE are the two open-ended captures: at least two
            // vertices (PLINE) / four fit points (SPLINE), more accepted,
            // finished with Enter.
            Self::Polyline => PointArity::AtLeast(2),
            Self::Spline => PointArity::AtLeast(4),
            // Start, a point on the arc, end; two axis endpoints plus the
            // other-axis distance; base, reference, target; or the two dimension
            // origins plus the dimension-line location.
            Self::Arc
            | Self::Ellipse
            | Self::Scale
            | Self::DimLinear
            | Self::DimAligned
            // DIMANGULAR: two entity picks plus the dimension-arc location.
            | Self::DimAngular => PointArity::Exact(3),
            // ALIGN: the first source point, its destination, then the second
            // source point and its destination (ADR 0016 decision F). STRETCH: the
            // two opposite corners of the crossing window, then the base point and
            // the displacement point (ADR 0016 decision E).
            Self::Align | Self::Stretch => PointArity::Exact(4),
            // One side/distance point, one point placement, the ARRAYPOLAR
            // centre, the single arc/circle pick of DIMCENTER, or TEXT's single
            // insertion point (its text is a typed string, not a point).
            Self::Offset
            | Self::Point
            | Self::ArrayPolar
            | Self::DimCenter
            | Self::Text => PointArity::Exact(1),
            // ARRAYRECT captures no points: it is defined entirely by the session
            // typed parameters plus the selection snapshot, so its complement is
            // the zero-point exact arity (ready as soon as the selection is
            // non-empty). HATCH has the same pre-selection shape: its boundary is
            // the selected entities and the parameters are the typed channel, so
            // it captures no points either.
            Self::ArrayRect | Self::Hatch => PointArity::Exact(0),
            // Two endpoints, centre+radius point, two opposite rectangle
            // corners, a move/copy delta, target+boundary, or a rotate angle /
            // mirror axis.
            Self::Line
            | Self::Circle
            | Self::Rectang
            | Self::Polygon
            | Self::Move
            | Self::Trim
            | Self::Copy
            | Self::Extend
            | Self::Rotate
            | Self::Mirror
            | Self::Chamfer
            | Self::Fillet
            // DIMRADIUS/DIMDIAMETER: the entity pick plus the dimension-line
            // location.
            | Self::DimRadius
            | Self::DimDiameter
            // DIMORDINATE: the feature point and the leader endpoint.
            | Self::DimOrdinate => PointArity::Exact(2),
        }
    }

    /// Minimum number of points this operation needs to be defined.
    ///
    /// For a fixed-arity (`Exact`) kind this is the exact complement; for an
    /// open-ended (`AtLeast`) kind (PLINE, SPLINE) it is the minimum needed to
    /// confirm, with more points still accepted.
    pub fn required_points(self) -> usize {
        self.arity().minimum()
    }

    /// Whether this operation needs a non-empty selection before it can run.
    ///
    /// MOVE transforms existing entities, COPY duplicates them, and
    /// ROTATE/SCALE/MIRROR transform (or mirror) them; all refuse to run with no
    /// selection rather than acting on nothing. OFFSET needs exactly one selected
    /// source (enforced by the command layer). HATCH fills the selected boundary
    /// entities, so it too needs a non-empty selection before it can run.
    pub fn requires_selection(self) -> bool {
        matches!(
            self,
            Self::Move
                | Self::Copy
                | Self::Rotate
                | Self::Scale
                | Self::Mirror
                | Self::Offset
                | Self::ArrayRect
                | Self::ArrayPolar
                | Self::Align
                | Self::Hatch
        )
    }

    /// How many **leading** capture steps pick an existing entity rather than a
    /// free point.
    ///
    /// TRIM/EXTEND and CHAMFER/FILLET pick an entity at **every** step, so this is
    /// the whole complement. DIMRADIUS/DIMDIAMETER/DIMCENTER pick one entity first
    /// and (for the radial pair) then take a free location point, so this is `1`.
    /// Every other kind captures free points only (`0`).
    ///
    /// The capture state machine reads this to route each step: a click is an
    /// entity pick while `points.len() < picked_step_count()`, and a free point
    /// after it. It replaces the old "all steps are picks" boolean, which could not
    /// express a mixed capture.
    pub fn picked_step_count(self) -> usize {
        match self {
            Self::Trim | Self::Extend | Self::Chamfer | Self::Fillet => self.required_points(),
            Self::DimRadius | Self::DimDiameter | Self::DimCenter => 1,
            // DIMANGULAR's two-line form picks both lines; its third step is the
            // free dimension-arc location.
            Self::DimAngular => 2,
            _ => 0,
        }
    }

    /// Whether this operation captures **any** existing entity (an entity pick) as
    /// a step.
    ///
    /// TRIM/EXTEND are the two AutoCAD commands whose arguments are object
    /// selections, not arbitrary coordinates: the click is an entity pick, and
    /// an empty-space click is refused instead of being accepted as a point and
    /// resolved later. CHAMFER/FILLET share that shape (each step picks a straight
    /// line). DIMRADIUS/DIMDIAMETER/DIMCENTER add the radial family, whose **only**
    /// entry point is an arc/circle pick (ADR 0017 decision C): their first step is
    /// an entity pick even though a later step may be a free point.
    ///
    /// These kinds resolve their refs at capture time, so they are only ready once
    /// every entity-pick step has a real entity behind it.
    pub fn needs_picked_entities(self) -> bool {
        self.picked_step_count() > 0
    }

    /// Whether this operation's **last** step is a typed string rather than a
    /// point (ADR 0019 decision A.1).
    ///
    /// Only TEXT is a text tool today. This is the single authority the capture
    /// state machine, the status projection (`awaiting_text`) and the UI prompt
    /// phase read; a UI must never infer "needs text" from `key` plus the point
    /// count, because `ready` alone cannot tell "missing points" from "missing
    /// text".
    pub fn requires_text(self) -> bool {
        matches!(self, Self::Text)
    }
}
