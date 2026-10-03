//! Interactive drawing/editing tool: point capture, preview and commit intent.
//!
//! Spec v2.0 §4.3/§4.6/§4.7 and `docs/drawing-edit.md` §2/§4. This is the
//! drawing counterpart of [`crate::measure_tool`] / [`crate::annotation_tool`]:
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

use cad_domain::{CadError, CadResult, Point3};

/// The drawing/editing operation a tool captures for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawToolKind {
    /// LINE: two points (start, end).
    Line,
    /// CIRCLE: centre plus a point on the circumference (the radius point).
    Circle,
    /// MOVE: requires a selection, then a two-point delta.
    Move,
    /// TRIM: a target pick plus a boundary pick.
    Trim,
}

impl DrawToolKind {
    /// Every kind, in the order the UI selector presents them. This is the one
    /// authoritative ordering; the Slint shell mirrors it instead of keeping a
    /// second hand-written list that can drift.
    pub const ALL: [DrawToolKind; 4] = [
        DrawToolKind::Line,
        DrawToolKind::Circle,
        DrawToolKind::Move,
        DrawToolKind::Trim,
    ];

    /// Stable, locale-independent machine key (never translated).
    pub fn key(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Circle => "circle",
            Self::Move => "move",
            Self::Trim => "trim",
        }
    }

    /// Parse a machine key back to a kind; unknown keys are rejected instead of
    /// silently falling back to a default operation.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "line" => Some(Self::Line),
            "circle" => Some(Self::Circle),
            "move" => Some(Self::Move),
            "trim" => Some(Self::Trim),
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
            Self::Move => "移动",
            Self::Trim => "修剪",
        }
    }

    /// Exact number of points this operation needs to be defined.
    pub fn required_points(self) -> usize {
        match self {
            // Two endpoints, centre+radius point, a move delta, or target+boundary.
            Self::Line | Self::Circle | Self::Move | Self::Trim => 2,
        }
    }

    /// Whether this operation needs a non-empty selection before it can run.
    ///
    /// Only MOVE transforms existing entities; the others create or edit from
    /// picks. A MOVE with no selection is refused rather than moving nothing.
    pub fn requires_selection(self) -> bool {
        matches!(self, Self::Move)
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
    /// Translate the current selection by `delta`.
    Move { delta: Point3 },
    /// Trim `target_pick` against `boundary_pick`.
    Trim {
        target_pick: Point3,
        boundary_pick: Point3,
    },
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
            DrawToolKind::Move => DrawIntent::Move {
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
        })
    }
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
        assert_eq!(DrawToolKind::from_key("arc"), None);
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
            assert_eq!(kind.required_points(), 2, "{kind:?}");
        }
        assert!(!DrawToolKind::Line.requires_selection());
        assert!(!DrawToolKind::Circle.requires_selection());
        assert!(DrawToolKind::Move.requires_selection());
        assert!(!DrawToolKind::Trim.requires_selection());
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
