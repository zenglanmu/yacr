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

use cad_domain::{CadError, CadResult, Point3, Transform3};

/// The drawing/editing operation a tool captures for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawToolKind {
    /// LINE: two points (start, end).
    Line,
    /// CIRCLE: centre plus a point on the circumference (the radius point).
    Circle,
    /// ARC: three points (start, a point on the arc, end).
    Arc,
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
}

impl DrawToolKind {
    /// Every kind, in the order the UI selector presents them. This is the one
    /// authoritative ordering; the Slint shell mirrors it instead of keeping a
    /// second hand-written list that can drift.
    pub const ALL: [DrawToolKind; 10] = [
        DrawToolKind::Line,
        DrawToolKind::Circle,
        DrawToolKind::Arc,
        DrawToolKind::Move,
        DrawToolKind::Trim,
        DrawToolKind::Copy,
        DrawToolKind::Extend,
        DrawToolKind::Rotate,
        DrawToolKind::Scale,
        DrawToolKind::Mirror,
    ];

    /// Stable, locale-independent machine key (never translated).
    pub fn key(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Circle => "circle",
            Self::Arc => "arc",
            Self::Move => "move",
            Self::Trim => "trim",
            Self::Copy => "copy",
            Self::Extend => "extend",
            Self::Rotate => "rotate",
            Self::Scale => "scale",
            Self::Mirror => "mirror",
        }
    }

    /// Parse a machine key back to a kind; unknown keys are rejected instead of
    /// silently falling back to a default operation.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "line" => Some(Self::Line),
            "circle" => Some(Self::Circle),
            "arc" => Some(Self::Arc),
            "move" => Some(Self::Move),
            "trim" => Some(Self::Trim),
            "copy" => Some(Self::Copy),
            "extend" => Some(Self::Extend),
            "rotate" => Some(Self::Rotate),
            "scale" => Some(Self::Scale),
            "mirror" => Some(Self::Mirror),
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
            Self::Move => "移动",
            Self::Trim => "修剪",
            Self::Copy => "复制",
            Self::Extend => "延伸",
            Self::Rotate => "旋转",
            Self::Scale => "缩放",
            Self::Mirror => "镜像",
        }
    }

    /// Exact number of points this operation needs to be defined.
    pub fn required_points(self) -> usize {
        match self {
            // Two endpoints, centre+radius point, a move/copy delta,
            // target+boundary, or a rotate angle / mirror axis (two points).
            Self::Line
            | Self::Circle
            | Self::Move
            | Self::Trim
            | Self::Copy
            | Self::Extend
            | Self::Rotate
            | Self::Mirror => 2,
            // Start, a point on the arc, end; or base, reference, target.
            Self::Arc | Self::Scale => 3,
        }
    }

    /// Whether this operation needs a non-empty selection before it can run.
    ///
    /// MOVE transforms existing entities, COPY duplicates them, and
    /// ROTATE/SCALE/MIRROR transform (or mirror) them; all refuse to run with no
    /// selection rather than acting on nothing.
    pub fn requires_selection(self) -> bool {
        matches!(
            self,
            Self::Move | Self::Copy | Self::Rotate | Self::Scale | Self::Mirror
        )
    }
}

/// Snapshot of an in-progress draw for the UI preview layer.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawPreview {
    pub kind: DrawToolKind,
    /// Captured points in order.
    pub points: Vec<Point3>,
    /// Live cursor position, if any; never committed until confirmed.
    pub cursor: Option<Point3>,
    /// Captured points still needed before the operation is defined.
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

    /// One-line, locale-facing capture step for the tool panel.
    ///
    /// Kept in `cad-app` so the Slint layer never re-implements the state
    /// machine's counting rules.
    pub fn status_line(&self) -> String {
        if self.kind.requires_selection() && self.selection_count == 0 {
            return format!("{}：请先选择对象", self.kind.label());
        }
        if self.kind == DrawToolKind::Trim {
            return match self.points.len() {
                0 => "修剪：请选择要修剪的目标".to_string(),
                1 => "修剪：请选择边界".to_string(),
                _ => "修剪：已选目标与边界，可确认".to_string(),
            };
        }
        if self.kind == DrawToolKind::Extend {
            return match self.points.len() {
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
/// It carries only geometry captured from the user. `Move` carries the delta
/// (the command layer supplies the refs from the session selection); `Trim`
/// carries the two pick points (the command layer resolves them to
/// `SelectionRef`s against the database, exactly as `docs/drawing-edit.md` §2
/// requires).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawIntent {
    /// Insert a LINE from `start` to `end` on the active layer.
    Line { start: Point3, end: Point3 },
    /// Insert a CIRCLE centred at `center` through `edge` (radius = |edge−center|).
    Circle { center: Point3, edge: Point3 },
    /// Insert an ARC through three points (start, on-arc, end).
    Arc { points: [Point3; 3] },
    /// Translate the current selection by `delta`.
    Move { delta: Point3 },
    /// Duplicate the current selection translated by `delta`.
    Copy { delta: Point3 },
    /// Trim `target_pick` against `boundary_pick`.
    Trim {
        target_pick: Point3,
        boundary_pick: Point3,
    },
    /// Extend `target_pick` to `boundary_pick`.
    Extend {
        target_pick: Point3,
        boundary_pick: Point3,
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
}

/// Drawing/editing capture state machine.
#[derive(Debug, Clone)]
pub struct DrawTool {
    kind: DrawToolKind,
    points: Vec<Point3>,
    cursor: Option<Point3>,
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
            cursor: None,
            selection_count,
        }
    }

    pub fn kind(&self) -> DrawToolKind {
        self.kind
    }

    pub fn points(&self) -> &[Point3] {
        &self.points
    }

    pub fn cursor(&self) -> Option<Point3> {
        self.cursor
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

    /// Capture one point, refusing non-finite input and overflow past the exact
    /// complement rather than silently resetting or dropping it.
    pub fn push_point(&mut self, point: Point3) -> CadResult<()> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput("draw point must be finite".into()));
        }
        if self.points.len() >= self.kind.required_points() {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' already has its {} points",
                self.kind.key(),
                self.kind.required_points()
            )));
        }
        self.points.push(point);
        Ok(())
    }

    pub fn remaining(&self) -> usize {
        self.kind
            .required_points()
            .saturating_sub(self.points.len())
    }

    /// Whether the captured parameters define the operation.
    pub fn is_ready(&self) -> bool {
        self.remaining() == 0 && (!self.kind.requires_selection() || self.selection_count > 0)
    }

    pub fn preview(&self) -> DrawPreview {
        DrawPreview {
            kind: self.kind,
            points: self.points.clone(),
            cursor: self.cursor,
            remaining: self.remaining(),
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
        if self.remaining() > 0 {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' needs {} point(s), has {}",
                self.kind.key(),
                self.kind.required_points(),
                self.points.len()
            )));
        }
        let a = self.points[0];
        let b = self.points[1];
        Ok(match self.kind {
            DrawToolKind::Line => DrawIntent::Line { start: a, end: b },
            DrawToolKind::Circle => DrawIntent::Circle { center: a, edge: b },
            DrawToolKind::Arc => DrawIntent::Arc {
                points: [a, b, self.points[2]],
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
                target_pick: a,
                boundary_pick: b,
            },
            DrawToolKind::Extend => DrawIntent::Extend {
                target_pick: a,
                boundary_pick: b,
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
        })
    }
}

/// Minimum XY extent for a gesture that defines a direction (rotate) or an axis
/// (mirror): a shorter span has no well-defined angle or line and is refused.
const MIN_DIRECTION_EXTENT: f64 = 1e-12;

/// Euclidean distance between two points.
fn distance(a: Point3, b: Point3) -> f64 {
    (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z)
}

/// Rotation about `base` by `angle` radians in the XY plane (about the +Z axis).
///
/// Built as `T(base) ∘ R(angle) ∘ T(−base)` through `matrix_mul` (right-hand side
/// applied first) and `from_basis` for the orthonormal rotation.
pub(crate) fn rotation_transform(base: Point3, angle: f64) -> Transform3 {
    let (sin, cos) = angle.sin_cos();
    let to_origin = Transform3::translation(Point3 {
        x: -base.x,
        y: -base.y,
        z: -base.z,
    });
    let back = Transform3::translation(base);
    let rotation = Transform3::from_basis(
        Point3 {
            x: cos,
            y: sin,
            z: 0.0,
        },
        Point3 {
            x: -sin,
            y: cos,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    back.matrix_mul(&rotation).matrix_mul(&to_origin)
}

/// Uniform scale about `base` by `factor`, built as
/// `T(base) ∘ S(factor) ∘ T(−base)`.
pub(crate) fn scale_transform(base: Point3, factor: f64) -> Transform3 {
    let to_origin = Transform3::translation(Point3 {
        x: -base.x,
        y: -base.y,
        z: -base.z,
    });
    let back = Transform3::translation(base);
    back.matrix_mul(&Transform3::scale(factor))
        .matrix_mul(&to_origin)
}

/// Reflection across the line through `p1` and `p2` (in the XY plane).
///
/// The reflection is a negative-determinant affine, so it cannot be expressed
/// with `from_basis` (which is always right-handed); the 2×2 reflection is built
/// directly and translated to pass through `p1`. A zero-length axis is refused.
pub(crate) fn mirror_transform(p1: Point3, p2: Point3) -> CadResult<Transform3> {
    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    let len = dx.hypot(dy);
    if !len.is_finite() || len <= 1e-12 {
        return Err(CadError::InvalidInput(
            "mirror axis needs two distinct points".into(),
        ));
    }
    let (ux, uy) = (dx / len, dy / len);
    let linear = [
        [ux * ux - uy * uy, 2.0 * ux * uy, 0.0],
        [2.0 * ux * uy, uy * uy - ux * ux, 0.0],
        [0.0, 0.0, 1.0],
    ];
    let mut matrix = [[0.0f64; 4]; 4];
    for r in 0..3 {
        for c in 0..3 {
            matrix[r][c] = linear[r][c];
        }
    }
    matrix[3][3] = 1.0;
    // Translation column = p1 − M·p1, so the reflection passes through p1.
    let mapped = [
        linear[0][0] * p1.x + linear[0][1] * p1.y + linear[0][2] * p1.z,
        linear[1][0] * p1.x + linear[1][1] * p1.y + linear[1][2] * p1.z,
        linear[2][0] * p1.x + linear[2][1] * p1.y + linear[2][2] * p1.z,
    ];
    matrix[0][3] = p1.x - mapped[0];
    matrix[1][3] = p1.y - mapped[1];
    matrix[2][3] = p1.z - mapped[2];
    Ok(Transform3 { matrix })
}

/// The centre and radius point of a CIRCLE intent, if the radius is positive.
///
/// A zero-radius circle is refused here so the UI can show the explicit
/// invalid-input reason before emitting a command; the database validates it
/// again.
pub fn circle_radius(center: Point3, edge: Point3) -> CadResult<f64> {
    let dx = edge.x - center.x;
    let dy = edge.y - center.y;
    let dz = edge.z - center.z;
    let radius = (dx * dx + dy * dy + dz * dz).sqrt();
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CadError::InvalidInput(
            "circle radius must be positive and finite".into(),
        ));
    }
    Ok(radius)
}

/// The centre, radius and signed angular span of the arc through three XY points.
///
/// The arc starts at `start`, passes through `through` and ends at `end`, on the
/// work-plane `+Z` normal. `sweep` is signed (negative means clockwise) so a
/// reflected capture is not silently re-drawn as its complement. Non-finite,
/// non-coplanar (different z) or collinear/coincident points are an explicit
/// refusal so no degenerate or silently-wrong 3D arc is ever created.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcParameters {
    pub center: Point3,
    pub radius: f64,
    /// Start angle in radians about `+Z`.
    pub start: f64,
    /// Signed sweep in radians; `|sweep| < 2π`.
    pub sweep: f64,
}

/// Compute the arc (centre/radius/start/sweep) through three points.
///
/// The circumcircle is solved in a frame translated by `start`, which makes both
/// the degeneracy test and the centre arithmetic translation- and
/// scale-invariant: a valid arc at large world coordinates is neither refused
/// nor computed with catastrophic cancellation. The operation is the documented
/// flat XY subset: points whose `z` differs from `start.z` beyond tolerance are
/// refused rather than projected into a silently-wrong arc. Collinear, coincident
/// or non-finite points are refused; the caller must not fabricate a centre or a
/// default sweep.
pub fn arc_parameters(start: Point3, through: Point3, end: Point3) -> CadResult<ArcParameters> {
    if !finite(start) || !finite(through) || !finite(end) {
        return Err(CadError::InvalidInput("arc points must be finite".into()));
    }
    // XY-only subset: a 3-point arc is a planar construction. Refuse a
    // non-coplanar pickup rather than silently projecting it.
    let z_tol = 1e-9 * (1.0 + start.z.abs() + through.z.abs() + end.z.abs());
    if (through.z - start.z).abs() > z_tol || (end.z - start.z).abs() > z_tol {
        return Err(CadError::InvalidInput(
            "arc points must lie in one XY plane".into(),
        ));
    }
    // Translate to `start` so the circumcircle is computed in a local frame.
    let a = (through.x - start.x, through.y - start.y);
    let b = (end.x - start.x, end.y - start.y);
    let a_len = a.0.hypot(a.1);
    let b_len = b.0.hypot(b.1);
    if a_len <= 1e-12 || b_len <= 1e-12 {
        return Err(CadError::InvalidInput("arc points are coincident".into()));
    }
    // Translation- and scale-invariant collinearity test (relative cross product).
    let cross = a.0 * b.1 - a.1 * b.0;
    if !cross.is_finite() || cross.abs() <= 1e-12 * a_len * b_len {
        return Err(CadError::InvalidInput("arc points are collinear".into()));
    }
    // Local circumcentre: `|C| = |a - C| = |b - C|` gives `a·C = |a|²/2`,
    // `b·C = |b|²/2`; solving by Cramer's rule yields the offset from `start`.
    let a_sq = a.0 * a.0 + a.1 * a.1;
    let b_sq = b.0 * b.0 + b.1 * b.1;
    let denom = 2.0 * cross;
    let offset_x = (a_sq * b.1 - b_sq * a.1) / denom;
    let offset_y = (a.0 * b_sq - b.0 * a_sq) / denom;
    let center = Point3 {
        x: start.x + offset_x,
        y: start.y + offset_y,
        z: start.z,
    };
    let radius = offset_x.hypot(offset_y);
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CadError::InvalidInput(
            "arc radius must be positive and finite".into(),
        ));
    }
    let angle = |p: Point3| (p.y - center.y).atan2(p.x - center.x);
    let start_angle = angle(start);
    let through_angle = angle(through);
    let end_angle = angle(end);
    let norm = |x: f64| {
        let tau = std::f64::consts::TAU;
        let x = x % tau;
        if x < 0.0 {
            x + tau
        } else {
            x
        }
    };
    let ccw = norm(end_angle - start_angle);
    let to_through = norm(through_angle - start_angle);
    // If the middle point is not on the counter-clockwise path, the arc runs
    // clockwise: a negative sweep, never the complementary CCW arc.
    let sweep = if to_through <= ccw {
        ccw
    } else {
        ccw - std::f64::consts::TAU
    };
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return Err(CadError::InvalidInput(
            "arc points are collinear or coincident".into(),
        ));
    }
    Ok(ArcParameters {
        center,
        radius,
        start: start_angle,
        sweep,
    })
}

fn finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn keys_indices_and_labels_round_trip_and_reject_unknowns() {
        for (index, kind) in DrawToolKind::ALL.iter().copied().enumerate() {
            assert_eq!(kind.index(), index);
            assert_eq!(DrawToolKind::from_index(index as i32), Some(kind));
            assert_eq!(DrawToolKind::from_key(kind.key()), Some(kind));
            assert_eq!(DrawToolKind::from_label(kind.label()), Some(kind));
        }
        assert_eq!(DrawToolKind::from_key("arc"), Some(DrawToolKind::Arc));
        assert_eq!(DrawToolKind::from_key("elbow"), None);
        assert_eq!(DrawToolKind::from_index(-1), None);
        assert_eq!(
            DrawToolKind::from_index(DrawToolKind::ALL.len() as i32),
            None
        );
        assert_eq!(DrawToolKind::from_label("半径"), None);
    }

    #[test]
    fn required_points_and_selection_are_explicit_per_kind() {
        for kind in DrawToolKind::ALL {
            let expected = match kind {
                DrawToolKind::Arc | DrawToolKind::Scale => 3,
                _ => 2,
            };
            assert_eq!(kind.required_points(), expected, "{kind:?}");
        }
        assert!(!DrawToolKind::Line.requires_selection());
        assert!(!DrawToolKind::Circle.requires_selection());
        assert!(!DrawToolKind::Arc.requires_selection());
        assert!(DrawToolKind::Move.requires_selection());
        assert!(!DrawToolKind::Trim.requires_selection());
        assert!(DrawToolKind::Copy.requires_selection());
        assert!(!DrawToolKind::Extend.requires_selection());
        assert!(DrawToolKind::Rotate.requires_selection());
        assert!(DrawToolKind::Scale.requires_selection());
        assert!(DrawToolKind::Mirror.requires_selection());
    }

    #[test]
    fn rotate_scale_mirror_capture_their_parameters() {
        // ROTATE: angle is the absolute direction from base to the second point.
        let mut rotate = DrawTool::new(DrawToolKind::Rotate, 2);
        rotate.push_point(p(0.0, 0.0)).unwrap();
        rotate.push_point(p(0.0, 1.0)).unwrap();
        match rotate.intent().unwrap() {
            DrawIntent::Rotate { base, angle } => {
                assert_eq!(base, p(0.0, 0.0));
                assert!(
                    (angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
                    "{angle}"
                );
            }
            other => panic!("expected Rotate, got {other:?}"),
        }
        // A zero-length direction (or one differing only in Z) has no defined
        // angle: refuse it rather than silently committing a 0° no-op.
        let mut degenerate_rotate = DrawTool::new(DrawToolKind::Rotate, 2);
        degenerate_rotate.push_point(p(2.0, 2.0)).unwrap();
        degenerate_rotate.push_point(p(2.0, 2.0)).unwrap();
        assert!(matches!(
            degenerate_rotate.intent(),
            Err(CadError::InvalidInput(_))
        ));
        let mut z_only_rotate = DrawTool::new(DrawToolKind::Rotate, 2);
        z_only_rotate.push_point(p(2.0, 2.0)).unwrap();
        z_only_rotate
            .push_point(Point3 {
                x: 2.0,
                y: 2.0,
                z: 5.0,
            })
            .unwrap();
        assert!(matches!(
            z_only_rotate.intent(),
            Err(CadError::InvalidInput(_))
        ));

        // SCALE: factor = |target − base| / |reference − base|.
        let mut scale = DrawTool::new(DrawToolKind::Scale, 1);
        scale.push_point(p(0.0, 0.0)).unwrap();
        scale.push_point(p(1.0, 0.0)).unwrap();
        scale.push_point(p(3.0, 0.0)).unwrap();
        match scale.intent().unwrap() {
            DrawIntent::Scale { base, factor } => {
                assert_eq!(base, p(0.0, 0.0));
                assert!((factor - 3.0).abs() < 1e-12, "{factor}");
            }
            other => panic!("expected Scale, got {other:?}"),
        }
        // A reference coincident with the base is refused, not an infinite factor.
        let mut degenerate = DrawTool::new(DrawToolKind::Scale, 1);
        degenerate.push_point(p(2.0, 2.0)).unwrap();
        degenerate.push_point(p(2.0, 2.0)).unwrap();
        degenerate.push_point(p(5.0, 2.0)).unwrap();
        assert!(matches!(
            degenerate.intent(),
            Err(CadError::InvalidInput(_))
        ));

        // MIRROR: the axis is the two captured points; equal points are refused.
        let mut mirror = DrawTool::new(DrawToolKind::Mirror, 1);
        mirror.push_point(p(0.0, 0.0)).unwrap();
        mirror.push_point(p(1.0, 1.0)).unwrap();
        assert_eq!(
            mirror.intent().unwrap(),
            DrawIntent::Mirror {
                p1: p(0.0, 0.0),
                p2: p(1.0, 1.0)
            }
        );
        let mut flat = DrawTool::new(DrawToolKind::Mirror, 1);
        flat.push_point(p(3.0, 3.0)).unwrap();
        flat.push_point(p(3.0, 3.0)).unwrap();
        assert!(matches!(flat.intent(), Err(CadError::InvalidInput(_))));
    }

    #[test]
    fn rotate_scale_mirror_transforms_are_correct() {
        // Rotation by 90° about the origin sends (1,0) to (0,1).
        let r = rotation_transform(p(0.0, 0.0), std::f64::consts::FRAC_PI_2);
        let rotated = r.apply_point(p(1.0, 0.0));
        assert!(
            rotated.x.abs() < 1e-12 && (rotated.y - 1.0).abs() < 1e-12,
            "{rotated:?}"
        );

        // Scale by 2 about (1,1): (2,1) -> (3,1).
        let s = scale_transform(p(1.0, 1.0), 2.0);
        let scaled = s.apply_point(p(2.0, 1.0));
        assert!(
            (scaled.x - 3.0).abs() < 1e-12 && (scaled.y - 1.0).abs() < 1e-12,
            "{scaled:?}"
        );

        // Mirror across the diagonal y = x swaps x and y.
        let m = mirror_transform(p(0.0, 0.0), p(1.0, 1.0)).unwrap();
        let mirrored = m.apply_point(p(2.0, 0.0));
        assert!(
            (mirrored.x - 0.0).abs() < 1e-12 && (mirrored.y - 2.0).abs() < 1e-12,
            "{mirrored:?}"
        );
        // A degenerate axis is refused.
        assert!(matches!(
            mirror_transform(p(1.0, 1.0), p(1.0, 1.0)),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn arc_captures_three_points_and_confirms_an_arc_intent() {
        let mut tool = DrawTool::new(DrawToolKind::Arc, 0);
        assert_eq!(tool.remaining(), 3);
        tool.push_point(p(0.0, 0.0)).unwrap();
        tool.push_point(p(5.0, 5.0)).unwrap();
        assert_eq!(tool.remaining(), 1);
        assert!(!tool.is_ready());
        tool.push_point(p(10.0, 0.0)).unwrap();
        assert!(tool.is_ready());
        assert_eq!(
            tool.intent().unwrap(),
            DrawIntent::Arc {
                points: [p(0.0, 0.0), p(5.0, 5.0), p(10.0, 0.0)]
            }
        );
    }

    #[test]
    fn extend_walks_target_then_boundary() {
        let mut tool = DrawTool::new(DrawToolKind::Extend, 0);
        assert_eq!(tool.preview().status_line(), "延伸：请选择要延伸的目标");
        tool.push_point(p(9.0, 0.0)).unwrap();
        assert_eq!(tool.preview().status_line(), "延伸：请选择边界");
        tool.push_point(p(20.0, 0.0)).unwrap();
        assert!(tool.is_ready());
        assert_eq!(
            tool.intent().unwrap(),
            DrawIntent::Extend {
                target_pick: p(9.0, 0.0),
                boundary_pick: p(20.0, 0.0)
            }
        );
    }

    #[test]
    fn arc_parameters_resolve_a_circumcircle_and_refuse_degenerate_input() {
        // Semicircle through (0,0), (5,5), (10,0): centre (5,0), radius 5.
        let parameters = arc_parameters(p(0.0, 0.0), p(5.0, 5.0), p(10.0, 0.0)).unwrap();
        assert!((parameters.center.x - 5.0).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.center.y - 0.0).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.radius - 5.0).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.start - std::f64::consts::PI).abs() < 1e-9);
        // Through the upper point, the arc is clockwise: a negative sweep.
        assert!((parameters.sweep + std::f64::consts::PI).abs() < 1e-9);

        // Collinear points have no circumcircle.
        assert!(matches!(
            arc_parameters(p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)),
            Err(CadError::InvalidInput(_))
        ));
        // Coincident points are refused, never a degenerate arc.
        assert!(matches!(
            arc_parameters(p(0.0, 0.0), p(0.0, 0.0), p(1.0, 0.0)),
            Err(CadError::InvalidInput(_))
        ));
        // Non-finite points are refused.
        assert!(matches!(
            arc_parameters(p(0.0, 0.0), p(f64::NAN, 1.0), p(2.0, 0.0)),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn arc_parameters_are_translation_invariant_at_large_coordinates() {
        // The semicircle fixture translated by (1e6, 1e6): centre (1e6+5, 1e6), r=5.
        let base = 1.0e6;
        let parameters = arc_parameters(
            p(base, base),
            p(base + 5.0, base + 5.0),
            p(base + 10.0, base),
        )
        .unwrap();
        assert!(
            (parameters.center.x - (base + 5.0)).abs() < 1e-9,
            "{parameters:?}"
        );
        assert!((parameters.center.y - base).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.radius - 5.0).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.start - std::f64::consts::PI).abs() < 1e-9);
        assert!((parameters.sweep + std::f64::consts::PI).abs() < 1e-9);
        // The arc passes through the picked points: each is exactly one radius out.
        for point in [
            p(base, base),
            p(base + 5.0, base + 5.0),
            p(base + 10.0, base),
        ] {
            let distance = (point.x - parameters.center.x).hypot(point.y - parameters.center.y);
            assert!(
                (distance - parameters.radius).abs() < 1e-9,
                "{point:?} is not on the circumcircle"
            );
        }
    }

    #[test]
    fn arc_parameters_accept_a_unit_arc_at_large_coordinates() {
        // A unit-radius upper semicircle centred at (1e6, 1e6) must be accepted,
        // not refused by a translation-dependent degeneracy test.
        let base = 1.0e6;
        let parameters = arc_parameters(
            p(base + 1.0, base),
            p(base, base + 1.0),
            p(base - 1.0, base),
        )
        .unwrap();
        assert!((parameters.center.x - base).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.center.y - base).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.radius - 1.0).abs() < 1e-9, "{parameters:?}");
        // right -> top -> left is a counter-clockwise half turn (positive sweep).
        assert!(
            (parameters.sweep - std::f64::consts::PI).abs() < 1e-9,
            "{parameters:?}"
        );
    }

    #[test]
    fn arc_parameters_supports_a_near_full_arc() {
        let tau = std::f64::consts::TAU;
        let end_angle = tau - 0.1;
        let parameters = arc_parameters(
            p(1.0, 0.0),
            p(-1.0, 0.0),
            Point3 {
                x: end_angle.cos(),
                y: end_angle.sin(),
                z: 0.0,
            },
        )
        .unwrap();
        assert!((parameters.center.x).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.center.y).abs() < 1e-9, "{parameters:?}");
        assert!((parameters.radius - 1.0).abs() < 1e-9, "{parameters:?}");
        assert!(
            (parameters.sweep - (tau - 0.1)).abs() < 1e-9,
            "{parameters:?}"
        );
    }

    #[test]
    fn arc_parameters_refuse_non_coplanar_points() {
        // A point off the start plane must be refused, never silently projected.
        let through = Point3 {
            x: 1.0,
            y: 0.0,
            z: 5.0,
        };
        assert!(matches!(
            arc_parameters(p(0.0, 0.0), through, p(0.0, 1.0)),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn line_captures_two_points_and_confirms_a_line_intent() {
        let mut tool = DrawTool::new(DrawToolKind::Line, 0);
        assert_eq!(tool.remaining(), 2);
        assert!(!tool.is_ready());
        tool.push_point(p(0.0, 0.0)).unwrap();
        assert_eq!(tool.remaining(), 1);
        assert!(!tool.is_ready());
        tool.push_point(p(4.0, 3.0)).unwrap();
        assert!(tool.is_ready());
        assert!(tool.preview().can_confirm());
        assert_eq!(
            tool.intent().unwrap(),
            DrawIntent::Line {
                start: p(0.0, 0.0),
                end: p(4.0, 3.0)
            }
        );
    }

    #[test]
    fn circle_intent_uses_centre_and_edge_and_radius_is_positive() {
        let mut tool = DrawTool::new(DrawToolKind::Circle, 0);
        tool.push_point(p(1.0, 1.0)).unwrap();
        tool.push_point(p(4.0, 5.0)).unwrap();
        assert_eq!(
            tool.intent().unwrap(),
            DrawIntent::Circle {
                center: p(1.0, 1.0),
                edge: p(4.0, 5.0)
            }
        );
        assert_eq!(circle_radius(p(1.0, 1.0), p(4.0, 5.0)).unwrap(), 5.0);
        // Degenerate radius is explicit, not a fabricated zero circle.
        assert!(matches!(
            circle_radius(p(1.0, 1.0), p(1.0, 1.0)),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn move_requires_a_selection_and_computes_the_delta() {
        let mut tool = DrawTool::new(DrawToolKind::Move, 0);
        tool.push_point(p(1.0, 2.0)).unwrap();
        tool.push_point(p(4.0, 6.0)).unwrap();
        // Even with both points, a missing selection is not ready.
        assert!(!tool.is_ready());
        assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));
        assert_eq!(tool.preview().status_line(), "移动：请先选择对象");

        tool.set_selection_count(2);
        assert!(tool.is_ready());
        assert_eq!(
            tool.intent().unwrap(),
            DrawIntent::Move { delta: p(3.0, 4.0) }
        );
    }

    #[test]
    fn trim_walks_target_then_boundary() {
        let mut tool = DrawTool::new(DrawToolKind::Trim, 0);
        assert_eq!(tool.preview().status_line(), "修剪：请选择要修剪的目标");
        tool.push_point(p(1.0, 0.0)).unwrap();
        assert_eq!(tool.preview().status_line(), "修剪：请选择边界");
        tool.push_point(p(1.0, 5.0)).unwrap();
        assert!(tool.is_ready());
        assert_eq!(
            tool.intent().unwrap(),
            DrawIntent::Trim {
                target_pick: p(1.0, 0.0),
                boundary_pick: p(1.0, 5.0)
            }
        );
    }

    #[test]
    fn a_third_point_and_a_non_finite_point_are_refused_without_capture() {
        let mut tool = DrawTool::new(DrawToolKind::Line, 0);
        tool.push_point(p(0.0, 0.0)).unwrap();
        tool.push_point(p(1.0, 0.0)).unwrap();
        assert!(matches!(
            tool.push_point(p(2.0, 0.0)),
            Err(CadError::InvalidInput(_))
        ));
        assert_eq!(tool.points().len(), 2);

        let mut fresh = DrawTool::new(DrawToolKind::Line, 0);
        assert!(fresh.push_point(p(f64::NAN, 0.0)).is_err());
        assert!(fresh.points().is_empty());
    }

    #[test]
    fn intent_refuses_incomplete_capture() {
        let tool = DrawTool::new(DrawToolKind::Circle, 0);
        assert!(matches!(tool.intent(), Err(CadError::InvalidInput(_))));
    }

    #[test]
    fn preview_status_and_cursor_follow_the_state_machine() {
        let mut tool = DrawTool::new(DrawToolKind::Line, 0);
        assert_eq!(tool.preview().status_line(), "直线：已选 0 点，还需 2 点");
        tool.set_cursor(Some(p(5.0, 5.0)));
        assert_eq!(tool.cursor(), Some(p(5.0, 5.0)));
        // The cursor never counts as a captured point.
        assert!(tool.points().is_empty());
        tool.push_point(p(0.0, 0.0)).unwrap();
        assert_eq!(tool.preview().status_line(), "直线：已选 1 点，还需 1 点");
        tool.push_point(p(1.0, 1.0)).unwrap();
        assert_eq!(tool.preview().status_line(), "直线：已选 2 点，可确认");
    }
}
