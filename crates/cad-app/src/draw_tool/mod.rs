//! Interactive drawing/editing tool: point capture, preview and commit intent.
//!
//! Spec v2.0 §4.3/§4.6/§4.7 and `docs/drawing-edit.md` §2/§4. This is the
//! drawing counterpart of [`crate::measure_tool`]:
//! a pure, host-agnostic state machine that captures exactly the points a given
//! edit needs, reports the remaining steps through a preview, and describes the
//! commit as a [`DrawIntent`].
//!
//! The module deliberately does **not** build a [`crate::Command`] itself. The
//! application commands (`CommandId::CreateLine`, `CreateCircle`,
//! `MoveEntities`, `TrimEntity`) are owned by the command layer (workstream B),
//! and `Move`/`Trim` additionally need the session's `SelectionRef`s, which a
//! pure point-capture tool must not invent. [`DrawIntent`] is the stable,
//! transaction-free description the UI/host turns into exactly one command; the
//! command layer maps it as documented in `docs/drawing-edit.md` §2.
//!
//! Cancelling never opens a transaction (the tool holds no database handle), so
//! a cancelled draw always leaves the drawing and history untouched. Confirming
//! yields exactly one intent, which the caller commits through one transaction.

use cad_domain::{CadError, CadResult, Point3, SelectionRef};

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
    pub const ALL: [DrawToolKind; 14] = [
        DrawToolKind::Line,
        DrawToolKind::Circle,
        DrawToolKind::Arc,
        DrawToolKind::Polyline,
        DrawToolKind::Rectang,
        DrawToolKind::Ellipse,
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
            // PLINE is the one open-ended capture: at least two vertices, more
            // accepted, finished with Enter.
            Self::Polyline => PointArity::AtLeast(2),
            // Start, a point on the arc, end; two axis endpoints plus the
            // other-axis distance; or base, reference, target.
            Self::Arc | Self::Ellipse | Self::Scale => PointArity::Exact(3),
            // One side/distance point.
            Self::Offset => PointArity::Exact(1),
            // Two endpoints, centre+radius point, two opposite rectangle
            // corners, a move/copy delta, target+boundary, or a rotate angle /
            // mirror axis.
            Self::Line
            | Self::Circle
            | Self::Rectang
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
    /// For a fixed-arity (`Exact`) kind this is the exact complement; for the
    /// open-ended (`AtLeast`) PLINE it is the minimum needed to confirm, with
    /// more points still accepted.
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

/// Snapshot of an in-progress draw for the UI preview layer.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawPreview {
    pub kind: DrawToolKind,
    /// Captured points in order.
    pub points: Vec<Point3>,
    /// Entities resolved at capture time, in step order.
    ///
    /// Empty for the free-point kinds. For the entity-picking kinds
    /// ([`DrawToolKind::needs_picked_entities`]) this is one parallel entry per
    /// captured step, so readiness is honest: a TRIM/EXTEND with two raw points
    /// but fewer than two resolved entities is **not** confirmable.
    pub picks: Vec<SelectionRef>,
    /// Live cursor position, if any; never committed until confirmed.
    pub cursor: Option<Point3>,
    /// The entity under the **current cursor** for the pending capture step, or
    /// `None`.
    ///
    /// Resolved through the exact pick path a click uses, only while a step is
    /// pending, and never fabricated: it is presentation feedback for the
    /// follow-up highlight wiring, not a commit source.
    pub candidate: Option<SelectionRef>,
    /// Captured points still needed before the operation is defined.
    ///
    /// This is the *minimum* complement ([`PointArity::minimum`] minus the
    /// captured count), so it is `0` once the operation can be confirmed. For an
    /// **open-ended** tool a further point is still accepted while this is `0`;
    /// the honest projection is [`Self::remaining_points`] (which returns `None`
    /// there) — this raw count is retained as the legacy numeric field the
    /// `cad-host`/`cad-engine` wire shape reads.
    pub remaining: usize,
    /// Number of selected entities the tool was started with (MOVE needs > 0).
    pub selection_count: usize,
    /// Whether the operation can be committed right now.
    pub ready: bool,
}

impl DrawPreview {
    /// Whether the captured parameters can be committed right now.
    pub fn can_confirm(&self) -> bool {
        self.ready
    }

    /// The capture arity of the tool this preview describes.
    pub fn arity(&self) -> PointArity {
        self.kind.arity()
    }

    /// Whether this capture is open-ended (accepts points past its minimum).
    pub fn is_open_ended(&self) -> bool {
        self.kind.arity().is_open_ended()
    }

    /// Honest projection of how many more points the capture requires.
    ///
    /// `Some(k)` means exactly `k` more points are needed to become confirmable
    /// (`Some(0)` = the exact complement is complete). `None` means the minimum
    /// is met on an **open-ended** tool: more points are still accepted up to
    /// the hard cap and the user finishes with Enter, so there is no fixed
    /// "remaining" count to report (never a fabricated `0`).
    pub fn remaining_points(&self) -> Option<usize> {
        let minimum = self.kind.required_points();
        if self.points.len() < minimum {
            Some(minimum - self.points.len())
        } else if self.kind.arity().is_open_ended() {
            None
        } else {
            Some(0)
        }
    }

    /// One-line, locale-facing capture step for the tool panel.
    ///
    /// Kept in `cad-app` so the UI layer never re-implements the state
    /// machine's counting rules. The entity-picking kinds count *resolved
    /// entities* ([`Self::picks`]) rather than raw points, so the step text
    /// cannot claim a step that an empty-space click never resolved. The
    /// open-ended kinds report the captured count and the minimum to confirm.
    ///
    /// NOTE (pre-existing i18n deviation): the branches below hardcode Chinese.
    /// The open-ended branch adds **locale-neutral ASCII** text instead of more
    /// hardcoded copy, so this change does not widen that deviation.
    pub fn status_line(&self) -> String {
        if self.kind.arity().is_open_ended() {
            let minimum = self.kind.required_points();
            if self.points.len() < minimum {
                return format!(
                    "{}: {} point(s) captured, at least {} needed",
                    self.kind.key(),
                    self.points.len(),
                    minimum
                );
            }
            return format!(
                "{}: {} point(s) captured, press Enter to finish",
                self.kind.key(),
                self.points.len()
            );
        }
        if self.kind.requires_selection() && self.selection_count == 0 {
            return format!("{}：请先选择对象", self.kind.label());
        }
        if self.kind == DrawToolKind::Trim {
            return match self.picks.len() {
                0 => "修剪：请选择要修剪的目标".to_string(),
                1 => "修剪：请选择边界".to_string(),
                _ => "修剪：已选目标与边界，可确认".to_string(),
            };
        }
        if self.kind == DrawToolKind::Extend {
            return match self.picks.len() {
                0 => "延伸：请选择要延伸的目标".to_string(),
                1 => "延伸：请选择边界".to_string(),
                _ => "延伸：已选目标与边界，可确认".to_string(),
            };
        }
        if self.remaining > 0 {
            format!(
                "{}：已选 {} 点，还需 {} 点",
                self.kind.label(),
                self.points.len(),
                self.remaining
            )
        } else {
            format!(
                "{}：已选 {} 点，可确认",
                self.kind.label(),
                self.points.len()
            )
        }
    }
}

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
}

/// Drawing/editing capture state machine.
#[derive(Debug, Clone)]
pub struct DrawTool {
    kind: DrawToolKind,
    points: Vec<Point3>,
    /// Entity refs resolved at capture time, one per captured step for the
    /// entity-picking kinds (TRIM/EXTEND); empty otherwise.
    picks: Vec<SelectionRef>,
    cursor: Option<Point3>,
    /// Entity under the current cursor for the pending pick step, or `None`.
    candidate: Option<SelectionRef>,
    selection_count: usize,
}

impl DrawTool {
    /// Start a tool for `kind` with the current selection count.
    ///
    /// A selection-requiring kind is still created; [`Self::is_ready`] refuses
    /// to confirm until `selection_count > 0`, so the refusal is visible in the
    /// step text rather than hidden.
    pub fn new(kind: DrawToolKind, selection_count: usize) -> Self {
        Self {
            kind,
            points: Vec::new(),
            picks: Vec::new(),
            cursor: None,
            candidate: None,
            selection_count,
        }
    }

    pub fn kind(&self) -> DrawToolKind {
        self.kind
    }

    pub fn points(&self) -> &[Point3] {
        &self.points
    }

    /// The entity refs resolved at capture time (empty for free-point kinds).
    pub fn picks(&self) -> &[SelectionRef] {
        &self.picks
    }

    pub fn cursor(&self) -> Option<Point3> {
        self.cursor
    }

    /// The entity under the current cursor for the pending pick step, if any.
    pub fn candidate(&self) -> Option<&SelectionRef> {
        self.candidate.as_ref()
    }

    pub fn selection_count(&self) -> usize {
        self.selection_count
    }

    /// Record the current selection count (MOVE needs a non-empty selection).
    pub fn set_selection_count(&mut self, count: usize) {
        self.selection_count = count;
    }

    /// Move the preview cursor without capturing a point.
    pub fn set_cursor(&mut self, cursor: Option<Point3>) {
        self.cursor = cursor;
    }

    /// Record the entity under the cursor for the pending pick step.
    ///
    /// Presentation feedback only: it never advances the capture and is cleared
    /// when the next step's pick is captured. Passing `None` is the honest "no
    /// entity under the cursor" state, never a fabricated ref.
    pub fn set_candidate(&mut self, candidate: Option<SelectionRef>) {
        self.candidate = candidate;
    }

    /// Whether the pending step is an entity pick (TRIM/EXTEND with a step left).
    pub fn requires_entity_pick(&self) -> bool {
        self.kind.needs_picked_entities() && self.points.len() < self.kind.required_points()
    }

    /// Capture one free point, refusing non-finite input and overflow past the
    /// exact complement (or the open-ended hard cap) rather than silently
    /// resetting or dropping it.
    ///
    /// The entity-picking kinds refuse here: their steps are entity picks, so a
    /// blind point that cannot carry an entity must never be captured as one
    /// ([`Self::push_pick`] is the only path for them). Once both TRIM/EXTEND
    /// steps are resolved the tool is ready and waiting for confirmation, so a
    /// further click is refused with **that** reason instead of the misleading
    /// "captures entity picks, not free points" (which describes a step that is
    /// still un-captured). The refusal is a machine reason, never a silent drop.
    pub fn push_point(&mut self, point: Point3) -> CadResult<()> {
        if self.kind.needs_picked_entities() {
            if self.points.len() >= self.kind.required_points() {
                return Err(CadError::InvalidInput(format!(
                    "draw kind '{}' is ready; press Enter to confirm",
                    self.kind.key()
                )));
            }
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' captures entity picks, not free points",
                self.kind.key()
            )));
        }
        self.push_point_checked(point)
    }

    /// Capture one entity pick: the raw point and the ref resolved for it.
    ///
    /// This is the atomic capture for the entity-picking kinds — the point and
    /// the entity it was resolved against are stored together, so the two can
    /// never drift. Free-point kinds refuse it.
    pub fn push_pick(&mut self, point: Point3, reference: SelectionRef) -> CadResult<()> {
        if !self.kind.needs_picked_entities() {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' does not take entity picks",
                self.kind.key()
            )));
        }
        self.push_point_checked(point)?;
        self.picks.push(reference);
        // The next step starts with no candidate; it is re-resolved on the next
        // cursor move rather than carried over from the previous step.
        self.candidate = None;
        Ok(())
    }

    /// Finite/overflow-checked point push shared by both capture paths.
    ///
    /// A fixed-arity kind refuses the `(n + 1)`-th point exactly as before; an
    /// open-ended kind accepts points up to [`MAX_OPEN_CAPTURE_POINTS`] and then
    /// refuses explicitly rather than growing without bound.
    fn push_point_checked(&mut self, point: Point3) -> CadResult<()> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput("draw point must be finite".into()));
        }
        match self.kind.arity() {
            PointArity::Exact(n) => {
                if self.points.len() >= n {
                    return Err(CadError::InvalidInput(format!(
                        "draw kind '{}' already has its {} points",
                        self.kind.key(),
                        n
                    )));
                }
            }
            PointArity::AtLeast(_) => {
                if self.points.len() >= MAX_OPEN_CAPTURE_POINTS {
                    return Err(CadError::InvalidInput(format!(
                        "draw kind '{}' has reached its maximum of {} points",
                        self.kind.key(),
                        MAX_OPEN_CAPTURE_POINTS
                    )));
                }
            }
        }
        self.points.push(point);
        Ok(())
    }

    /// How many more points the capture still requires, honestly per arity.
    ///
    /// `Some(k)`: exactly `k` more points are needed before the tool is
    /// confirmable (identical to the old fixed-arity complement for every fixed
    /// kind). `None`: the minimum is met on an **open-ended** tool — more points
    /// are still accepted and the capture is finished with Enter, so there is no
    /// fixed count to report (never a fabricated `0`).
    pub fn remaining(&self) -> Option<usize> {
        let minimum = self.kind.required_points();
        if self.points.len() < minimum {
            Some(minimum - self.points.len())
        } else if self.kind.arity().is_open_ended() {
            None
        } else {
            Some(0)
        }
    }

    /// Whether the captured parameters define the operation.
    ///
    /// The entity-picking kinds additionally require a resolved ref for **every**
    /// step, so two raw points with a missed pick are never confirmable.
    pub fn is_ready(&self) -> bool {
        self.points.len() >= self.kind.required_points()
            && (!self.kind.requires_selection() || self.selection_count > 0)
            && (!self.kind.needs_picked_entities()
                || self.picks.len() >= self.kind.required_points())
    }

    pub fn preview(&self) -> DrawPreview {
        DrawPreview {
            kind: self.kind,
            points: self.points.clone(),
            picks: self.picks.clone(),
            cursor: self.cursor,
            candidate: self.candidate.clone(),
            // The legacy numeric field is the minimum complement; the honest
            // open-ended projection lives in `DrawPreview::remaining_points`.
            remaining: self.remaining().unwrap_or(0),
            selection_count: self.selection_count,
            ready: self.is_ready(),
        }
    }

    /// The exact commit this tool describes, or an explicit error when a
    /// parameter is still missing. Never fabricates a point or a default delta.
    pub fn intent(&self) -> CadResult<DrawIntent> {
        if self.kind.requires_selection() && self.selection_count == 0 {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' needs a non-empty selection",
                self.kind.key()
            )));
        }
        if self.points.len() < self.kind.required_points() {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' needs at least {} point(s), has {}",
                self.kind.key(),
                self.kind.required_points(),
                self.points.len()
            )));
        }
        if self.kind.needs_picked_entities() && self.picks.len() < self.kind.required_points() {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' needs {} resolved entity pick(s), has {}",
                self.kind.key(),
                self.kind.required_points(),
                self.picks.len()
            )));
        }
        let a = self.points[0];
        // One-point tools (OFFSET) never index a second point; `unwrap_or(a)` is a
        // harmless placeholder only ever used when a kind's arm does not read `b`.
        let b = self.points.get(1).copied().unwrap_or(a);
        Ok(match self.kind {
            DrawToolKind::Line => DrawIntent::Line { start: a, end: b },
            DrawToolKind::Circle => DrawIntent::Circle { center: a, edge: b },
            DrawToolKind::Arc => DrawIntent::Arc {
                points: [a, b, self.points[2]],
            },
            // PLINE: every captured vertex, in order. The open-ended capture is
            // refused above while below the minimum, so `points` has at least
            // the two vertices the command layer requires.
            DrawToolKind::Polyline => DrawIntent::Polyline {
                points: self.points.clone(),
            },
            // RECTANG: the two captured corners exactly as clicked. The
            // geometry-side refusal (non-finite, degenerate, non-coplanar) and
            // the canonical vertex ordering live in the command layer, so the
            // edit tool's capture never invents or reorders a corner.
            DrawToolKind::Rectang => DrawIntent::Rect {
                corner1: a,
                corner2: b,
            },
            // ELLIPSE: the three captured picks exactly as clicked, in order. The
            // geometry-side construction, axis swap and degeneracy refusals live
            // in the command layer, so the tool never invents a centre or ratio.
            DrawToolKind::Ellipse => DrawIntent::Ellipse {
                p1: a,
                p2: b,
                p3: self.points[2],
            },
            DrawToolKind::Move => DrawIntent::Move {
                delta: Point3 {
                    x: b.x - a.x,
                    y: b.y - a.y,
                    z: b.z - a.z,
                },
            },
            DrawToolKind::Copy => DrawIntent::Copy {
                delta: Point3 {
                    x: b.x - a.x,
                    y: b.y - a.y,
                    z: b.z - a.z,
                },
            },
            DrawToolKind::Trim => DrawIntent::Trim {
                target: self.picks[0].clone(),
                boundary: self.picks[1].clone(),
                pick_point: a,
            },
            DrawToolKind::Extend => DrawIntent::Extend {
                target: self.picks[0].clone(),
                boundary: self.picks[1].clone(),
                pick_point: a,
            },
            // ROTATE: the absolute direction angle from `base` to the second
            // point, measured about +Z from the +X axis. A zero-length direction
            // (or one differing only in Z) has no defined angle, so it is refused
            // rather than silently becoming a 0° no-op.
            DrawToolKind::Rotate => {
                if (b.x - a.x).hypot(b.y - a.y) <= MIN_DIRECTION_EXTENT {
                    return Err(CadError::InvalidInput(
                        "rotate direction point must differ from the base".into(),
                    ));
                }
                let angle = (b.y - a.y).atan2(b.x - a.x);
                if !angle.is_finite() {
                    return Err(CadError::InvalidInput(
                        "rotate direction must define a finite angle".into(),
                    ));
                }
                DrawIntent::Rotate { base: a, angle }
            }
            // SCALE: factor = |target − base| / |reference − base|; a degenerate
            // reference (== base) is an explicit refusal, never an infinite scale.
            DrawToolKind::Scale => {
                let reference = distance(a, b);
                if !reference.is_finite() || reference <= 0.0 {
                    return Err(CadError::InvalidInput(
                        "scale reference point must differ from the base".into(),
                    ));
                }
                let target = distance(a, self.points[2]);
                let factor = target / reference;
                if !factor.is_finite() || factor <= 0.0 {
                    return Err(CadError::InvalidInput(
                        "scale factor must be positive and finite".into(),
                    ));
                }
                DrawIntent::Scale { base: a, factor }
            }
            // MIRROR: the axis is the line through the two points; an axis of zero
            // length is refused rather than reflecting across a made-up line.
            DrawToolKind::Mirror => {
                if (b.x - a.x).hypot(b.y - a.y) <= MIN_DIRECTION_EXTENT {
                    return Err(CadError::InvalidInput(
                        "mirror axis needs two distinct points".into(),
                    ));
                }
                DrawIntent::Mirror { p1: a, p2: b }
            }
            // OFFSET: the single captured point is the side/distance point; the
            // source is the session selection, resolved by the command layer.
            DrawToolKind::Offset => DrawIntent::Offset { side_point: a },
        })
    }
}

/// Minimum XY extent for a gesture that defines a direction (rotate) or an axis
/// (mirror): a shorter span has no well-defined angle or line and is refused.
const MIN_DIRECTION_EXTENT: f64 = 1e-12;

/// Hard cap on an open-ended capture (PLINE), in points.
///
/// An interactive polyline is digitised a handful of vertices at a time; 1024 is
/// far beyond any hand-placed screen/plan run (and ≈32 KiB of `Point3`, a small
/// bounded footprint), while the bound stops a stuck pointer or a synthetic
/// click loop from growing the capture without limit. Reaching it is an explicit
/// refusal, never a silent drop of the extra click.
pub const MAX_OPEN_CAPTURE_POINTS: usize = 1024;

/// Euclidean distance between two points.
fn distance(a: Point3, b: Point3) -> f64 {
    (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z)
}
mod geometry;
// Re-exported so the historical `crate::draw_tool::<name>` paths and the public
// `cad_app::circle_radius` (via `lib.rs`) stay valid for the callers that used the
// pre-split module layout.
pub use geometry::{
    arc_parameters, circle_radius, ellipse_parameters, ArcParameters, EllipseParameters,
};
pub(crate) use geometry::{mirror_transform, rotation_transform, scale_transform};

#[cfg(test)]
mod tests;
