//! The commit contract a confirmed capture produces (ADR 0015 decisions A/B).
//!
//! Split out of [`super`] to keep the state-machine module inside the code-size
//! limit. [`DrawIntent`] is re-exported by [`super`], so `cad_app::DrawIntent`,
//! `crate::draw_tool::DrawIntent` and the crate-internal paths are unchanged.
//!
//! The intent is a pure description: it never holds a database handle and never
//! commits. The command layer turns exactly one intent into exactly one
//! transaction, and the session owns any value that is not part of what the user
//! clicked (for example POLYGON's side count, ALIGN's scale flag and TEXT's
//! height), which is why those never appear as fields here.

use cad_domain::{Point3, SelectionRef};

// The catalog constants named by the doc links below are not used at runtime; the
// links are kept fully qualified so this module needs no import for them.

/// The pure description of a commit the command layer turns into exactly one
/// drawing transaction.
///
/// It carries the geometry captured from the user. `Move` carries the delta
/// (the command layer supplies the refs from the session selection); `Trim` and
/// `Extend` carry the `SelectionRef`s resolved **at capture time** alongside the
/// raw `pick_point` the command needs (which side to trim, which end to extend).
/// The command layer must not re-pick at confirm: the refs here are the single
/// truth for what the user saw under the cursor.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawIntent {
    /// Insert a LINE from `start` to `end` on the active layer.
    Line { start: Point3, end: Point3 },
    /// Insert a CIRCLE centred at `center` through `edge` (radius = |edge−center|).
    Circle { center: Point3, edge: Point3 },
    /// Insert an ARC through three points (start, on-arc, end).
    Arc { points: [Point3; 3] },
    /// Insert an open polyline through `points` (at least two), on the active
    /// layer. Always `closed == false`, one zero bulge per vertex (straight
    /// segments only); see [`DrawToolKind::Polyline`] for the honest subset.
    Polyline { points: Vec<Point3> },
    /// Insert a closed four-vertex rectangle whose opposite corners are
    /// `corner1` and `corner2`, on the active layer.
    ///
    /// The two points are the two opposite corners the user clicked, in click
    /// order; they do **not** define a winding, so the command layer canonicalises
    /// them (see `rectangle_geometry`). Always `closed == true` with one zero bulge
    /// per vertex; see [`DrawToolKind::Rectang`] for the honest subset.
    Rect { corner1: Point3, corner2: Point3 },
    /// Insert a full ELLIPSE from the three axis-endpoint picks: the two endpoints
    /// `p1`/`p2` of the first axis and the third pick `p3` giving the other
    /// half-axis distance, on the active layer.
    ///
    /// The three points are exactly the three clicks, in order; the geometry-side
    /// construction (`ellipse_parameters`), the axis swap and every degeneracy
    /// refusal live in the command layer.
    Ellipse { p1: Point3, p2: Point3, p3: Point3 },
    /// Insert a closed regular polygon **inscribed** in the circle through
    /// `center` (pick 1) and `radius_point` (pick 2), on the active layer.
    ///
    /// The side count is the session sysvar (ADR 0015 decision C), not a field
    /// of this intent: the command layer reads it from the session so the intent
    /// stays the pure description of what the user clicked. The two points are
    /// exactly the two clicks, in order; the geometry-side construction
    /// ([`polygon_vertices`]) and every degeneracy refusal live in the command
    /// layer. Always `closed == true` with one zero bulge per vertex.
    Polygon {
        center: Point3,
        radius_point: Point3,
    },
    /// Place a single POINT at `position` on the active layer.
    ///
    /// `position` is the one click, exactly as captured; the command layer stores
    /// `SemanticGeometry::Point(position)` and refuses a non-finite position. A
    /// single point has no coplanarity constraint, so there is no `+Z` refusal
    /// here. AutoCAD's repeat-until-Enter placement is NOT IMPLEMENTED.
    Point { position: Point3 },
    /// Insert a fit-point SPLINE through `fit_points` (at least four), on the
    /// active layer.
    ///
    /// `fit_points` are the captured fit points in click order; the interpolation
    /// (degree 3, clamped chord knots, one control point per fit point) and every
    /// degeneracy refusal live in the command layer
    /// ([`crate::draw_tool::spline_control_points`]), so the tool never invents a
    /// control point or a knot.
    Spline { fit_points: Vec<Point3> },
    /// Translate the current selection by `delta`.
    Move { delta: Point3 },
    /// Duplicate the current selection translated by `delta`.
    Copy { delta: Point3 },
    /// Trim `target` against `boundary`; `pick_point` selects the kept side.
    Trim {
        target: SelectionRef,
        boundary: SelectionRef,
        pick_point: Point3,
    },
    /// Extend `target` to `boundary`; `pick_point` selects the extended end.
    Extend {
        target: SelectionRef,
        boundary: SelectionRef,
        pick_point: Point3,
    },
    /// Rotate the current selection about `base` by `angle` radians (about +Z).
    Rotate { base: Point3, angle: f64 },
    /// Scale the current selection about `base` by the positive `factor`.
    Scale { base: Point3, factor: f64 },
    /// Reflect the current selection across the line through `p1` and `p2`,
    /// keeping the originals and adding mirrored copies.
    ///
    /// The axis is the line in the XY plane; only `p1.x/p1.y` and `p2.x/p2.y`
    /// define it, so a vertical-looking axis that differs only in Z is refused
    /// (see `mirror_transform`). Z is carried through unchanged.
    Mirror { p1: Point3, p2: Point3 },
    /// Offset the single selected LINE/CIRCLE; `side_point` gives both the side
    /// and the distance.
    Offset { side_point: Point3 },
    /// CHAMFER the two picked straight lines (ADR 0016 decision C).
    ///
    /// `first`/`second` are the refs resolved at capture time, in pick order;
    /// `pick_first`/`pick_second` are the raw click points that select each pick
    /// side. The two distances are the session typed channel, read by the command
    /// layer — never carried here.
    Chamfer {
        first: SelectionRef,
        second: SelectionRef,
        pick_first: Point3,
        pick_second: Point3,
    },
    /// FILLET the two picked straight lines (ADR 0016 decision B); same shape as
    /// [`DrawIntent::Chamfer`], with the radius from the session typed channel.
    Fillet {
        first: SelectionRef,
        second: SelectionRef,
        pick_first: Point3,
        pick_second: Point3,
    },
    /// ARRAYRECT the current selection on a rectangular grid (ADR 0016 decision D).
    ///
    /// Carries no field: the grid is defined entirely by the session typed
    /// parameters ([`super::ToolParameter::ArrayRows`] … `ArrayColumnSpacing`) and the
    /// selection snapshot the caller supplies, so the intent stays the pure
    /// description of what the user asked for (the source's bounding-box
    /// lower-left base point is derived in the command layer).
    ArrayRect,
    /// ARRAYPOLAR the current selection around `center` (ADR 0016 decision D).
    ///
    /// `center` is the one captured point; the item count, fill angle and
    /// rotate-items flag are the session typed channel, read by the command layer.
    ArrayPolar { center: Point3 },
    /// ALIGN the current selection from two point pairs (ADR 0016 decision F).
    ///
    /// The four points are exactly the four clicks in AutoCAD's capture order
    /// (`source1`, `dest1`, then `source2`, `dest2`); the `AlignScale` flag is the
    /// session typed channel ([`super::ToolParameter::AlignScale`]) read by the command
    /// layer, so it never rides the intent. The selection refs travel separately
    /// (the caller supplies the snapshot), and the composed transform is applied
    /// **in place** — no copies.
    Align {
        source1: Point3,
        dest1: Point3,
        source2: Point3,
        dest2: Point3,
    },
    /// STRETCH the entities inside a crossing window (ADR 0016 decision E).
    ///
    /// The four points are the four clicks in AutoCAD's order: the two opposite
    /// corners of the crossing window (`rect`), the base point, then the
    /// displacement point. All four are **world** points exactly as captured;
    /// the effective translation is `delta − base`. The window is always
    /// CROSSING and the enclosure classification is recomputed by the command
    /// layer from the rectangle and the geometry (never carried here).
    Stretch {
        rect: [Point3; 2],
        base: Point3,
        delta: Point3,
    },
    /// DIMLINEAR: insert a horizontal or vertical linear dimension through the
    /// two extension-line origins `first`/`second` and the dimension-line
    /// location `location`, on the active layer (ADR 0017 decision C, P5c).
    ///
    /// The three points are exactly the three clicks, in order. The command layer
    /// decides horizontal vs vertical from `location` (AutoCAD's rule), builds
    /// the definition and every refusal lives there, so the tool never invents a
    /// direction or a measurement.
    DimLinear {
        first: Point3,
        second: Point3,
        location: Point3,
    },
    /// DIMALIGNED: insert a dimension whose line is parallel to the line joining
    /// the two origins, through `first`/`second`/`location` on the active layer
    /// (ADR 0017 decision C, P5c).
    ///
    /// The three points are exactly the three clicks, in order; the command layer
    /// measures the true origin-to-origin distance and owns every refusal.
    DimAligned {
        first: Point3,
        second: Point3,
        location: Point3,
    },
    /// DIMRADIUS: insert a radius dimension for the picked arc/circle `target`,
    /// with the leader/dimension-line direction set by the `location` pick, on the
    /// active layer (ADR 0017 decision C, P5d).
    ///
    /// `target` is the ref resolved at capture time (the **only** entry point);
    /// `location` is the second pick, a free point whose direction from the circle
    /// centre sets where the leader meets the circle. The command layer resolves
    /// the entity, builds the definition and owns every refusal (a non-arc/circle
    /// pick, an off-plane entity/location, a degenerate radius, a location at the
    /// centre), so the tool never invents a radius or a direction.
    DimRadius {
        target: SelectionRef,
        location: Point3,
    },
    /// DIMDIAMETER: insert a diameter dimension for the picked arc/circle
    /// `target`, same two-step capture as [`DrawIntent::DimRadius`] (ADR 0017
    /// decision C, P5d).
    ///
    /// The command layer stores the **opposite chord point** in the group-10 slot
    /// and measures the diameter; every refusal matches DIMRADIUS.
    DimDiameter {
        target: SelectionRef,
        location: Point3,
    },
    /// DIMCENTER: create the non-associative centre mark/centrelines of the picked
    /// arc/circle `target` as **plain geometry** (ADR 0017 decision C/D.7, P5d).
    ///
    /// `target` is the ref resolved at capture time; the command layer resolves the
    /// entity, reads the resolved `DIMCEN`, produces ordinary LINE geometry (or,
    /// with `DIMCEN = 0`, nothing plus a diagnostic and no transaction) and owns
    /// every refusal. This is **not** a `DIMENSION` entity.
    DimCenter { target: SelectionRef },
    /// DIMANGULAR: insert an angular dimension between the two picked straight
    /// lines, with `location` selecting the dimensioned angle (ADR 0017 decision C,
    /// P5e).
    ///
    /// `first`/`second` are the refs resolved at capture time (the vertex is their
    /// intersection); `location` is the dimension-arc pick. The command layer
    /// resolves the lines, builds the definition and owns every refusal, so the
    /// capture never invents an intersection or an angle.
    DimAngular {
        first: SelectionRef,
        second: SelectionRef,
        location: Point3,
    },
    /// DIMORDINATE: insert an ordinate dimension for `feature` with the leader
    /// ending at `leader` (ADR 0017 decision C, P5e).
    ///
    /// The axis follows the command layer's auto rule (the interactive form never
    /// forces one); every refusal lives there.
    DimOrdinate { feature: Point3, leader: Point3 },
    /// DIMBASELINE: stack one linear dimension per picked point off the base
    /// dimension `base` (ADR 0017 decision C, P5f).
    ///
    /// `base` is the ref resolved at pick time (never a remembered "last
    /// dimension"); `points` are the captured points in pick order, i.e. every
    /// capture point **after** the base pick. The row construction (rotation,
    /// stacking normal, `i · DIMDLI` offsets) and every refusal live in the
    /// command layer, so the capture never invents a stacking direction. All N
    /// created dimensions commit in one transaction / one undo step.
    DimBaseline {
        base: SelectionRef,
        points: Vec<Point3>,
    },
    /// DIMCONTINUE: chain one linear dimension per picked point from the base
    /// dimension `base` (ADR 0017 decision C, P5f).
    ///
    /// Same shape as [`DrawIntent::DimBaseline`]; the chain construction (segment
    /// 1 starts at the base's second origin, every segment stays on the base's
    /// dimension line, a non-advancing segment is refused) lives in the command
    /// layer.
    DimContinue {
        base: SelectionRef,
        points: Vec<Point3>,
    },
    /// HATCH the current selection (ADR 0018 decision D, P6b).
    ///
    /// Carries no field: the boundary is the selection snapshot the caller
    /// supplies, and the pattern type/spacing/angle/double/style are the session
    /// typed channel ([`super::ToolParameter::HatchPatternType`] … `HatchStyle`), so the
    /// intent stays the pure description of what the user asked for. This kind
    /// captures **no points** (the zero-point exact complement) and takes no
    /// in-command picks.
    Hatch,
    /// TEXT: place `text` at the captured insertion point (ADR 0019 F.1).
    ///
    /// The height is the session typed channel
    /// ([`super::ToolParameter::TextHeight`], default `2.5`), read by the command layer,
    /// so the intent stays the pure description of what the user supplied. The
    /// text is always a non-empty, at most
    /// [`super::MAX_TOOL_TEXT_CHARS`]-character string validated at capture time.
    Text { position: Point3, text: String },
    /// MLEADER: place `text` on a straight-line leader from `arrow` to the
    /// `landing` content-connection point (ADR 0019 decision F.2, P7c).
    ///
    /// The two points are exactly the two clicks, in order; the dogleg start, the
    /// dogleg/arrow/text sizes and the attachments are the command layer's
    /// derivation from the resolved `MLeaderStyle` table (`MultiLeaderGeometry::created`),
    /// so the intent never carries a style value and cannot contradict the table.
    /// The text is validated at capture time (non-empty, at most
    /// [`super::MAX_TOOL_TEXT_CHARS`] characters) and re-validated by the command.
    MLeader {
        arrow: Point3,
        landing: Point3,
        text: String,
    },
    /// TEXTEDIT: replace the stored string of `target` with `text` (ADR 0020
    /// amendment A).
    ///
    /// `target` is the ref resolved **at capture time** (never re-picked at
    /// confirm). The command re-reads that entity and refuses by name when it is
    /// gone or no longer a single, directly addressable `Text`; it replaces only
    /// the string, leaving style/height/rotation/position/alignment untouched.
    /// The text is validated at capture time (non-empty, at most
    /// [`super::MAX_TOOL_TEXT_CHARS`] characters) and re-validated by the
    /// command.
    TextEdit { target: SelectionRef, text: String },
}
