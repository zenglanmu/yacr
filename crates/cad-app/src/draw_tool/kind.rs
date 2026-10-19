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
}

impl DrawToolKind {
    /// Every kind, in the order the UI selector presents them. This is the one
    /// authoritative ordering; the UI shell mirrors it instead of keeping a
    /// second hand-written list that can drift.
    pub const ALL: [DrawToolKind; 17] = [
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
            // other-axis distance; or base, reference, target.
            Self::Arc | Self::Ellipse | Self::Scale => PointArity::Exact(3),
            // One side/distance point, or one point placement.
            Self::Offset | Self::Point => PointArity::Exact(1),
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
            | Self::Mirror => PointArity::Exact(2),
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
    /// source (enforced by the command layer).
    pub fn requires_selection(self) -> bool {
        matches!(
            self,
            Self::Move | Self::Copy | Self::Rotate | Self::Scale | Self::Mirror | Self::Offset
        )
    }

    /// Whether each capture step of this operation picks an **existing entity**
    /// rather than a free point.
    ///
    /// TRIM and EXTEND are the two AutoCAD commands whose arguments are object
    /// selections, not arbitrary coordinates: the click is an entity pick, and
    /// an empty-space click is refused instead of being accepted as a point and
    /// resolved later. These kinds therefore resolve their refs at capture time
    /// and are only ready once both steps have a real entity behind them.
    pub fn needs_picked_entities(self) -> bool {
        matches!(self, Self::Trim | Self::Extend)
    }
}
