//! Interactive measurement tool: point capture, preview and cancel.
//!
//! Spec v2.0 §3.3 (tasks F06/U04). The tool selects its algorithm from the
//! active tool kind instead of guessing from the raw point count, so a
//! three-point polyline is never silently measured as an angle. Measurement is
//! read-only and opens no database transaction, so cancelling produces zero
//! transactions by construction.

use cad_db::MeasurementAlgorithm;
use cad_domain::Point3;

/// Algorithm a measurement tool is capturing for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementToolKind {
    Distance,
    PolylineLength,
    Angle,
    Area,
}

impl MeasurementToolKind {
    /// Every kind, in the order the UI selector presents them. This is the one
    /// authoritative ordering; the Slint panel mirrors it instead of keeping a
    /// second hand-written list that can drift.
    pub const ALL: [MeasurementToolKind; 4] = [
        MeasurementToolKind::Distance,
        MeasurementToolKind::PolylineLength,
        MeasurementToolKind::Angle,
        MeasurementToolKind::Area,
    ];

    /// Stable, locale-independent machine key (never translated).
    pub fn key(self) -> &'static str {
        match self {
            Self::Distance => "distance",
            Self::PolylineLength => "polyline",
            Self::Angle => "angle",
            Self::Area => "area",
        }
    }

    /// Parse a machine key back to a kind; unknown keys are rejected instead of
    /// silently falling back to a default algorithm.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "distance" => Some(Self::Distance),
            "polyline" => Some(Self::PolylineLength),
            "angle" => Some(Self::Angle),
            "area" => Some(Self::Area),
            _ => None,
        }
    }

    /// Resolve a kind from its user-facing label (as displayed by a combobox).
    ///
    /// The labels come from [`Self::label`], so the UI model and this lookup
    /// share one source of truth. Unknown labels are rejected.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.label() == label)
    }

    /// Position in [`Self::ALL`], for a combobox `current-index`.
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

    /// The engine algorithm this tool evaluates with.
    pub fn algorithm(self) -> MeasurementAlgorithm {
        match self {
            Self::Distance => MeasurementAlgorithm::Distance3d,
            Self::PolylineLength => MeasurementAlgorithm::PolylineLength,
            Self::Angle => MeasurementAlgorithm::Angle3Points,
            Self::Area => MeasurementAlgorithm::PlanarPolygonArea,
        }
    }

    /// Points required before an evaluation is defined.
    pub fn min_points(self) -> usize {
        match self {
            Self::Distance | Self::PolylineLength => 2,
            Self::Angle | Self::Area => 3,
        }
    }

    /// Exact point count for tools that commit as soon as they are complete.
    pub fn exact_points(self) -> Option<usize> {
        match self {
            Self::Distance => Some(2),
            Self::Angle => Some(3),
            Self::PolylineLength | Self::Area => None,
        }
    }

    /// Whether the tool commits automatically once it has enough points.
    pub fn auto_completes(self) -> bool {
        self.exact_points().is_some()
    }

    /// Short user-facing label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Distance => "距离",
            Self::PolylineLength => "折线长度",
            Self::Angle => "角度",
            Self::Area => "面积",
        }
    }
}

/// Snapshot of an in-progress measurement for the UI preview layer.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementPreview {
    pub kind: MeasurementToolKind,
    /// Captured points in order.
    pub points: Vec<Point3>,
    /// Live cursor position, if any; never part of a result until captured.
    pub cursor: Option<Point3>,
    /// Captured points still needed before the measurement is defined.
    pub remaining: usize,
    /// Whether the current capture can be evaluated. Open-ended tools are still
    /// `ready` only after an explicit confirm.
    pub ready: bool,
}

impl MeasurementPreview {
    /// Whether the captured points can be committed right now.
    ///
    /// Open-ended tools (polyline/area) are only confirmable after an explicit
    /// confirm; auto-completing tools commit as soon as they are ready. The UI
    /// uses this to enable its confirm affordance without guessing.
    pub fn can_confirm(&self) -> bool {
        self.ready
    }

    /// One-line, locale-facing capture step for the tool panel.
    ///
    /// Kept in `cad-app` so the Slint layer never re-implements the state
    /// machine's counting rules.
    pub fn status_line(&self) -> String {
        if self.remaining > 0 {
            format!(
                "{}：已选 {} 点，还需 {} 点",
                self.kind.label(),
                self.points.len(),
                self.remaining
            )
        } else if self.ready {
            format!(
                "{}：已选 {} 点，可确认",
                self.kind.label(),
                self.points.len()
            )
        } else {
            format!("{}：已选 {} 点", self.kind.label(), self.points.len())
        }
    }
}

/// Measurement tool state machine.
#[derive(Debug, Clone)]
pub struct MeasurementTool {
    kind: MeasurementToolKind,
    points: Vec<Point3>,
    cursor: Option<Point3>,
}

impl MeasurementTool {
    pub fn new(kind: MeasurementToolKind) -> Self {
        Self {
            kind,
            points: Vec::new(),
            cursor: None,
        }
    }

    pub fn kind(&self) -> MeasurementToolKind {
        self.kind
    }

    pub fn points(&self) -> &[Point3] {
        &self.points
    }

    pub fn cursor(&self) -> Option<Point3> {
        self.cursor
    }

    /// Move the preview cursor without capturing a point.
    pub fn set_cursor(&mut self, cursor: Option<Point3>) {
        self.cursor = cursor;
    }

    /// Capture one point. Capture is unbounded; [`Self::is_ready`] decides when
    /// an evaluation is defined.
    pub fn push_point(&mut self, point: Point3) {
        self.points.push(point);
    }

    pub fn is_ready(&self) -> bool {
        match self.kind.exact_points() {
            Some(exact) => self.points.len() == exact,
            None => self.points.len() >= self.kind.min_points(),
        }
    }

    /// True when the tool commits as soon as it is ready (no explicit confirm).
    pub fn auto_ready(&self) -> bool {
        self.kind.auto_completes() && self.is_ready()
    }

    pub fn remaining(&self) -> usize {
        self.kind.min_points().saturating_sub(self.points.len())
    }

    pub fn preview(&self) -> MeasurementPreview {
        MeasurementPreview {
            kind: self.kind,
            points: self.points.clone(),
            cursor: self.cursor,
            remaining: self.remaining(),
            ready: self.is_ready(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn each_kind_selects_its_own_algorithm() {
        assert_eq!(
            MeasurementToolKind::Distance.algorithm(),
            MeasurementAlgorithm::Distance3d
        );
        assert_eq!(
            MeasurementToolKind::PolylineLength.algorithm(),
            MeasurementAlgorithm::PolylineLength
        );
        assert_eq!(
            MeasurementToolKind::Angle.algorithm(),
            MeasurementAlgorithm::Angle3Points
        );
        assert_eq!(
            MeasurementToolKind::Area.algorithm(),
            MeasurementAlgorithm::PlanarPolygonArea
        );
    }

    #[test]
    fn point_capture_advances_the_preview_and_ready_state() {
        let mut tool = MeasurementTool::new(MeasurementToolKind::Angle);
        assert_eq!(tool.remaining(), 3);
        assert!(!tool.is_ready());
        tool.push_point(p(0.0, 0.0));
        assert_eq!(tool.remaining(), 2);
        tool.push_point(p(1.0, 0.0));
        assert_eq!(tool.remaining(), 1);
        assert!(!tool.is_ready());
        tool.push_point(p(0.0, 1.0));
        assert_eq!(tool.remaining(), 0);
        assert!(tool.is_ready());
        assert!(tool.auto_ready());
    }

    #[test]
    fn open_ended_tool_is_ready_but_not_auto_complete() {
        let mut tool = MeasurementTool::new(MeasurementToolKind::PolylineLength);
        tool.push_point(p(0.0, 0.0));
        tool.push_point(p(1.0, 0.0));
        assert!(tool.is_ready());
        assert!(!tool.auto_ready());
        assert_eq!(tool.preview().points.len(), 2);
    }

    #[test]
    fn kind_keys_round_trip_and_index_matches_all() {
        for (index, kind) in MeasurementToolKind::ALL.iter().copied().enumerate() {
            assert_eq!(kind.index(), index);
            assert_eq!(MeasurementToolKind::from_index(index as i32), Some(kind));
            assert_eq!(MeasurementToolKind::from_key(kind.key()), Some(kind));
        }
        // Unknown keys/indices are rejected, never silently defaulted.
        assert_eq!(MeasurementToolKind::from_key("radius"), None);
        assert_eq!(MeasurementToolKind::from_index(-1), None);
        assert_eq!(
            MeasurementToolKind::from_index(MeasurementToolKind::ALL.len() as i32),
            None
        );
        // The combobox labels are unique and resolve back to their kind.
        for kind in MeasurementToolKind::ALL {
            assert_eq!(MeasurementToolKind::from_label(kind.label()), Some(kind));
        }
        assert_eq!(MeasurementToolKind::from_label("半径"), None);
    }

    #[test]
    fn preview_status_and_confirm_follow_the_state_machine() {
        let mut tool = MeasurementTool::new(MeasurementToolKind::Angle);
        // Not enough points: cannot confirm, status still counts down.
        tool.push_point(p(0.0, 0.0));
        let preview = tool.preview();
        assert!(!preview.can_confirm());
        assert_eq!(preview.status_line(), "角度：已选 1 点，还需 2 点");

        // Ready: confirmable.
        tool.push_point(p(1.0, 0.0));
        tool.push_point(p(0.0, 1.0));
        let preview = tool.preview();
        assert!(preview.can_confirm());
        assert_eq!(preview.status_line(), "角度：已选 3 点，可确认");
    }

    #[test]
    fn open_ended_preview_is_confirmable_once_minimum_met() {
        let mut tool = MeasurementTool::new(MeasurementToolKind::Area);
        assert!(!tool.preview().can_confirm());
        for point in [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)] {
            tool.push_point(point);
        }
        assert!(tool.preview().can_confirm());
    }
}
