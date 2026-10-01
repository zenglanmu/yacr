//! Interactive annotation creation: parameter capture, preview, confirm, cancel.
//!
//! Spec v2.0 §3.4 (tasks F07/F08/F09). This is the annotation counterpart of
//! [`crate::measure_tool`]: a real tool state machine that captures exactly the
//! parameters a given annotation kind needs, reports the remaining steps through
//! a preview, and commits through the single annotation transaction/history path
//! on confirm. Cancelling never opens a transaction, so a cancelled annotation
//! always leaves the annotation database and history untouched.
//!
//! Six kinds are supported, matching the six [`AnnotationGeometry`] variants the
//! database can store: text, leader, rectangle, ellipse, freehand and revision
//! cloud. Every kind declares its required parameters explicitly
//! ([`AnnotationToolKind::required_points`], [`AnnotationToolKind::requires_text`])
//! so a commit is refused while a parameter is still missing; nothing is
//! defaulted into a "successful" annotation.

use cad_db::{Annotation, AnnotationGeometry, AnnotationStyle};
use cad_domain::{AnnotationId, CadError, CadResult, Point3, Precision, SpaceId};
use std::f64::consts::PI;

/// Maximum number of points a single annotation captures.
///
/// Freehand and cloud are open-ended; this bound stops an unbounded capture
/// session from growing without limit while still allowing real strokes.
pub const MAX_ANNOTATION_POINTS: usize = 4096;

/// The six annotation kinds the application can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationToolKind {
    Text,
    Leader,
    Rectangle,
    Ellipse,
    Freehand,
    Cloud,
}

impl AnnotationToolKind {
    /// Every kind, in the order the UI selector presents them. This is the one
    /// authoritative ordering; the Slint panel mirrors it.
    pub const ALL: [AnnotationToolKind; 6] = [
        AnnotationToolKind::Text,
        AnnotationToolKind::Leader,
        AnnotationToolKind::Rectangle,
        AnnotationToolKind::Ellipse,
        AnnotationToolKind::Freehand,
        AnnotationToolKind::Cloud,
    ];

    /// Stable, locale-independent machine key (never translated).
    pub fn key(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Leader => "leader",
            Self::Rectangle => "rectangle",
            Self::Ellipse => "ellipse",
            Self::Freehand => "freehand",
            Self::Cloud => "cloud",
        }
    }

    /// Parse a machine key back to a kind; unknown keys are rejected instead of
    /// silently falling back to a default kind.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "text" => Some(Self::Text),
            "leader" => Some(Self::Leader),
            "rectangle" => Some(Self::Rectangle),
            "ellipse" => Some(Self::Ellipse),
            "freehand" => Some(Self::Freehand),
            "cloud" => Some(Self::Cloud),
            _ => None,
        }
    }

    /// Resolve a kind from its user-facing label (as displayed by a combobox).
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

    /// Short user-facing label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文字",
            Self::Leader => "引线",
            Self::Rectangle => "矩形",
            Self::Ellipse => "椭圆",
            Self::Freehand => "自由线",
            Self::Cloud => "云线",
        }
    }

    /// Exact number of points this kind requires, or `None` when it is
    /// open-ended (freehand/cloud).
    pub fn exact_points(self) -> Option<usize> {
        match self {
            // Text is placed at one point; the label text is a separate step.
            Self::Text => Some(1),
            // Rectangle and ellipse are defined by two diagonal points.
            Self::Rectangle | Self::Ellipse => Some(2),
            // A leader needs a start (arrow/A-tip) and an end (text side).
            Self::Leader => Some(2),
            Self::Freehand | Self::Cloud => None,
        }
    }

    /// Minimum number of points before a commit is defined.
    pub fn min_points(self) -> usize {
        match self {
            Self::Text => 1,
            Self::Leader | Self::Rectangle | Self::Ellipse => 2,
            // A stroke needs at least two samples to be a meaningful path.
            Self::Freehand | Self::Cloud => 2,
        }
    }

    /// Whether this kind needs a text payload before it can be committed.
    ///
    /// Only text annotations carry user text; a leader also labels its end. All
    /// other kinds have no text and never require one.
    pub fn requires_text(self) -> bool {
        matches!(self, Self::Text | Self::Leader)
    }

    /// Whether the tool commits automatically once its points are complete.
    ///
    /// Only kinds without an open-ended capture and without a text step can
    /// auto-commit. Text and leader always wait for their text, so they are
    /// committed by an explicit confirm; freehand and cloud are explicit too.
    pub fn auto_completes(self) -> bool {
        self.exact_points().is_some() && !self.requires_text()
    }
}

/// Snapshot of an in-progress annotation for the UI preview layer.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationPreview {
    pub kind: AnnotationToolKind,
    /// Captured points in order.
    pub points: Vec<Point3>,
    /// Live cursor position, if any; never part of the annotation until the
    /// stroke is confirmed.
    pub cursor: Option<Point3>,
    /// Captured points still needed before the geometry is defined.
    pub remaining: usize,
    /// The text step: whether this kind needs text and it has been supplied.
    pub requires_text: bool,
    pub text_supplied: bool,
}

impl AnnotationPreview {
    /// Whether the geometry is defined (enough points for the kind).
    pub fn geometry_ready(&self) -> bool {
        self.remaining == 0
            && (self.points.len() >= self.kind.min_points() || !self.requires_points())
    }

    fn requires_points(&self) -> bool {
        true
    }

    /// Whether the whole annotation can be committed right now: enough points
    /// *and*, when required, a non-empty text payload.
    pub fn can_confirm(&self) -> bool {
        self.remaining == 0 && (!self.requires_text || self.text_supplied)
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
        } else if self.requires_text && !self.text_supplied {
            format!("{}：请输入文字后确认", self.kind.label())
        } else {
            format!(
                "{}：已选 {} 点，可确认",
                self.kind.label(),
                self.points.len()
            )
        }
    }
}

/// Annotation creation state machine.
#[derive(Debug, Clone)]
pub struct AnnotationTool {
    kind: AnnotationToolKind,
    points: Vec<Point3>,
    cursor: Option<Point3>,
    text: String,
}

impl AnnotationTool {
    pub fn new(kind: AnnotationToolKind) -> Self {
        Self {
            kind,
            points: Vec::new(),
            cursor: None,
            text: String::new(),
        }
    }

    pub fn kind(&self) -> AnnotationToolKind {
        self.kind
    }

    pub fn points(&self) -> &[Point3] {
        &self.points
    }

    pub fn cursor(&self) -> Option<Point3> {
        self.cursor
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Move the preview cursor without capturing a point.
    pub fn set_cursor(&mut self, cursor: Option<Point3>) {
        self.cursor = cursor;
    }

    /// Set/replace the text payload for kinds that need one.
    ///
    /// Rejected for kinds that do not carry text, so a stray text event can
    /// never turn e.g. a cloud into a text-bearing annotation by accident.
    pub fn set_text(&mut self, text: impl Into<String>) -> CadResult<()> {
        if !self.kind.requires_text() {
            return Err(CadError::InvalidInput(format!(
                "annotation kind '{}' does not take text",
                self.kind.key()
            )));
        }
        self.text = text.into();
        Ok(())
    }

    /// Capture one point. Capture is bounded by [`MAX_ANNOTATION_POINTS`] and
    /// refused once a fixed-point kind already has its full complement.
    pub fn push_point(&mut self, point: Point3) -> CadResult<()> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput(
                "annotation point must be finite".into(),
            ));
        }
        if let Some(exact) = self.kind.exact_points() {
            if self.points.len() >= exact {
                return Err(CadError::InvalidInput(format!(
                    "annotation kind '{}' already has its {} points",
                    self.kind.key(),
                    exact
                )));
            }
        } else if self.points.len() >= MAX_ANNOTATION_POINTS {
            return Err(CadError::InvalidInput(format!(
                "annotation capture is limited to {MAX_ANNOTATION_POINTS} points"
            )));
        }
        self.points.push(point);
        Ok(())
    }

    pub fn remaining(&self) -> usize {
        self.kind.min_points().saturating_sub(self.points.len())
    }

    /// Whether the points needed for this kind are captured.
    pub fn geometry_ready(&self) -> bool {
        self.points.len() >= self.kind.min_points()
    }

    /// True when the tool commits as soon as it has enough points (no explicit
    /// confirm and no text step).
    pub fn auto_ready(&self) -> bool {
        self.kind.auto_completes() && self.geometry_ready()
    }

    pub fn preview(&self) -> AnnotationPreview {
        AnnotationPreview {
            kind: self.kind,
            points: self.points.clone(),
            cursor: self.cursor,
            remaining: self.remaining(),
            requires_text: self.kind.requires_text(),
            text_supplied: !self.text.trim().is_empty(),
        }
    }

    /// Build the [`Annotation`] this tool describes, or explain what is missing.
    ///
    /// This performs the per-kind required-parameter check and the geometry
    /// construction; it never fabricates a default point, text or axis. The
    /// caller supplies the id/space/timestamps so the tool stays free of host
    /// identity policy.
    pub fn build(
        &self,
        id: AnnotationId,
        space: SpaceId,
        created_unix_ms: i64,
        modified_unix_ms: i64,
        style: AnnotationStyle,
    ) -> CadResult<Annotation> {
        let points = &self.points;
        if points.len() < self.kind.min_points() {
            return Err(CadError::InvalidInput(format!(
                "annotation '{}' needs {} point(s), has {}",
                self.kind.key(),
                self.kind.min_points(),
                points.len()
            )));
        }
        if self.kind.requires_text() && self.text.trim().is_empty() {
            return Err(CadError::InvalidInput(format!(
                "annotation '{}' needs a non-empty text payload",
                self.kind.key()
            )));
        }

        let geometry = match self.kind {
            AnnotationToolKind::Text => AnnotationGeometry::Text(points[0]),
            AnnotationToolKind::Leader => AnnotationGeometry::Leader(points.clone()),
            AnnotationToolKind::Rectangle => AnnotationGeometry::Rectangle([points[0], points[1]]),
            AnnotationToolKind::Ellipse => {
                let center = points[0];
                let corner = points[1];
                // Half-extents from the centre to the diagonal point define two
                // orthogonal axes on the work plane; degenerate axes are refused
                // by `validate_annotation` rather than invented.
                let axis_u = Point3 {
                    x: corner.x - center.x,
                    y: 0.0,
                    z: 0.0,
                };
                let axis_v = Point3 {
                    x: 0.0,
                    y: corner.y - center.y,
                    z: 0.0,
                };
                AnnotationGeometry::Ellipse {
                    center,
                    axis_u,
                    axis_v,
                }
            }
            AnnotationToolKind::Freehand => AnnotationGeometry::Freehand(points.clone()),
            AnnotationToolKind::Cloud => AnnotationGeometry::Cloud(points.clone()),
        };

        let annotation = Annotation {
            id,
            space,
            geometry,
            text: if self.kind.requires_text() {
                self.text.clone()
            } else {
                String::new()
            },
            style,
            created_unix_ms,
            modified_unix_ms,
            anchor: None,
            precision: Precision::Analytic,
        };
        Ok(annotation)
    }
}

/// The number of points needed for a closed stroke to enclose an area.
///
/// Freehand/cloud capture is open-ended; this is only a documentation helper for
/// the UI's "keep drawing" state, not a commit precondition.
pub fn stroke_is_closed(points: &[Point3]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let first = points[0];
    let last = points[points.len() - 1];
    let d = ((first.x - last.x).powi(2) + (first.y - last.y).powi(2) + (first.z - last.z).powi(2))
        .sqrt();
    d <= f64::EPSILON
}

/// The turn in radians between two successive segments, used to seed a cloud
/// revision (not part of commit validation).
pub fn segment_turn(a: Point3, b: Point3, c: Point3) -> Option<f64> {
    let u = Point3 {
        x: b.x - a.x,
        y: b.y - a.y,
        z: b.z - a.z,
    };
    let v = Point3 {
        x: c.x - b.x,
        y: c.y - b.y,
        z: c.z - b.z,
    };
    let lu = (u.x * u.x + u.y * u.y + u.z * u.z).sqrt();
    let lv = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if lu < 1e-12 || lv < 1e-12 {
        return None;
    }
    let dot = (u.x * v.x + u.y * v.y + u.z * v.z) / (lu * lv);
    // Signed turn magnitude: the angle between the segments. The sign is the
    // orientation of the turn about the plane normal; for a planar 2D stroke
    // this is the sign of the cross product's component along the common axis,
    // which we approximate by the cross product's z component (the dominant
    // axis for a drawing-plane stroke) with a stable fallback to its norm.
    let cross = Point3 {
        x: u.y * v.z - u.z * v.y,
        y: u.z * v.x - u.x * v.z,
        z: u.x * v.y - u.y * v.x,
    };
    let signed = if cross.z.abs() > 1e-12 {
        cross.z
    } else {
        // Out-of-plane turn: fall back to the axis with the largest component
        // so the sign is still well defined.
        let (ax, ay, az) = (cross.x.abs(), cross.y.abs(), cross.z.abs());
        if ax >= ay && ax >= az {
            cross.x
        } else if ay >= az {
            cross.y
        } else {
            cross.z
        }
    };
    let sign = if signed < 0.0 { -1.0 } else { 1.0 };
    Some(dot.clamp(-1.0, 1.0).acos() * sign)
}

/// The number of full turns a stroke makes about its own centroid.
///
/// This is a pure geometric measure of how "cloud-like" a closed stroke is; it
/// is exposed for tests/UI but never gates a commit.
pub fn winding_turns(points: &[Point3]) -> f64 {
    if points.len() < 4 {
        return 0.0;
    }
    let n = points.len() as f64;
    let cx = points.iter().map(|p| p.x).sum::<f64>() / n;
    let cy = points.iter().map(|p| p.y).sum::<f64>() / n;
    let mut total = 0.0;
    for w in points.windows(2) {
        let a = w[0];
        let b = w[1];
        let a1 = (a.y - cy).atan2(a.x - cx);
        let a2 = (b.y - cy).atan2(b.x - cx);
        let mut d = a2 - a1;
        while d > PI {
            d -= 2.0 * PI;
        }
        while d < -PI {
            d += 2.0 * PI;
        }
        total += d;
    }
    total / (2.0 * PI)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn build(tool: &AnnotationTool) -> CadResult<Annotation> {
        tool.build(
            AnnotationId(1),
            SpaceId::Model,
            0,
            0,
            AnnotationStyle::default(),
        )
    }

    #[test]
    fn keys_indices_and_labels_round_trip_and_reject_unknowns() {
        for (index, kind) in AnnotationToolKind::ALL.iter().copied().enumerate() {
            assert_eq!(kind.index(), index);
            assert_eq!(AnnotationToolKind::from_index(index as i32), Some(kind));
            assert_eq!(AnnotationToolKind::from_key(kind.key()), Some(kind));
            assert_eq!(AnnotationToolKind::from_label(kind.label()), Some(kind));
        }
        assert_eq!(AnnotationToolKind::from_key("circle"), None);
        assert_eq!(AnnotationToolKind::from_index(-1), None);
        assert_eq!(
            AnnotationToolKind::from_index(AnnotationToolKind::ALL.len() as i32),
            None
        );
        assert_eq!(AnnotationToolKind::from_label("半径"), None);
    }

    #[test]
    fn required_parameters_are_explicit_per_kind() {
        assert_eq!(AnnotationToolKind::Text.exact_points(), Some(1));
        assert_eq!(AnnotationToolKind::Rectangle.exact_points(), Some(2));
        assert_eq!(AnnotationToolKind::Ellipse.exact_points(), Some(2));
        assert_eq!(AnnotationToolKind::Leader.exact_points(), Some(2));
        assert_eq!(AnnotationToolKind::Freehand.exact_points(), None);
        assert_eq!(AnnotationToolKind::Cloud.exact_points(), None);

        assert!(AnnotationToolKind::Text.requires_text());
        assert!(AnnotationToolKind::Leader.requires_text());
        assert!(!AnnotationToolKind::Rectangle.requires_text());
        assert!(!AnnotationToolKind::Ellipse.requires_text());
        assert!(!AnnotationToolKind::Freehand.requires_text());
        assert!(!AnnotationToolKind::Cloud.requires_text());

        // Only the point-only fixed kinds auto-complete.
        assert!(AnnotationToolKind::Rectangle.auto_completes());
        assert!(AnnotationToolKind::Ellipse.auto_completes());
        assert!(!AnnotationToolKind::Text.auto_completes());
        assert!(!AnnotationToolKind::Leader.auto_completes());
        assert!(!AnnotationToolKind::Freehand.auto_completes());
        assert!(!AnnotationToolKind::Cloud.auto_completes());
    }

    #[test]
    fn a_fixed_point_kind_refuses_points_beyond_its_complement() {
        let mut tool = AnnotationTool::new(AnnotationToolKind::Rectangle);
        tool.push_point(p(0.0, 0.0)).unwrap();
        tool.push_point(p(1.0, 1.0)).unwrap();
        assert!(matches!(
            tool.push_point(p(2.0, 2.0)),
            Err(CadError::InvalidInput(_))
        ));
        assert_eq!(tool.points().len(), 2);
    }

    #[test]
    fn non_finite_points_are_refused_without_capture() {
        let mut tool = AnnotationTool::new(AnnotationToolKind::Freehand);
        assert!(tool.push_point(p(f64::NAN, 0.0)).is_err());
        assert!(tool.points().is_empty());
    }

    #[test]
    fn text_is_only_accepted_for_text_bearing_kinds() {
        let mut cloud = AnnotationTool::new(AnnotationToolKind::Cloud);
        assert!(cloud.set_text("no").is_err());
        let mut text = AnnotationTool::new(AnnotationToolKind::Text);
        assert!(text.set_text("批注").is_ok());
        assert_eq!(text.text(), "批注");
    }

    #[test]
    fn step_progression_and_preview_follow_the_state_machine() {
        let mut tool = AnnotationTool::new(AnnotationToolKind::Rectangle);
        assert_eq!(tool.remaining(), 2);
        assert!(!tool.preview().can_confirm());
        assert_eq!(tool.preview().status_line(), "矩形：已选 0 点，还需 2 点");
        tool.push_point(p(0.0, 0.0)).unwrap();
        assert_eq!(tool.remaining(), 1);
        assert_eq!(tool.preview().status_line(), "矩形：已选 1 点，还需 1 点");
        tool.push_point(p(4.0, 3.0)).unwrap();
        assert_eq!(tool.remaining(), 0);
        assert!(tool.geometry_ready());
        assert!(tool.auto_ready());
        let preview = tool.preview();
        assert!(preview.can_confirm());
        assert!(!preview.requires_text);
        assert_eq!(preview.status_line(), "矩形：已选 2 点，可确认");
    }

    #[test]
    fn text_kind_needs_both_a_point_and_text_before_confirm() {
        let mut tool = AnnotationTool::new(AnnotationToolKind::Text);
        assert!(!tool.preview().can_confirm());
        tool.push_point(p(1.0, 2.0)).unwrap();
        // Point captured but no text: still not confirmable.
        let preview = tool.preview();
        assert!(preview.requires_text);
        assert!(!preview.text_supplied);
        assert!(!preview.can_confirm());
        assert_eq!(preview.status_line(), "文字：请输入文字后确认");
        tool.set_text("hello").unwrap();
        assert!(tool.preview().can_confirm());
    }

    #[test]
    fn leader_needs_two_points_and_text() {
        let mut tool = AnnotationTool::new(AnnotationToolKind::Leader);
        tool.push_point(p(0.0, 0.0)).unwrap();
        assert!(!tool.preview().can_confirm());
        tool.push_point(p(1.0, 1.0)).unwrap();
        assert!(!tool.preview().can_confirm());
        tool.set_text("note").unwrap();
        assert!(tool.preview().can_confirm());
        let ann = build(&tool).unwrap();
        assert_eq!(ann.text, "note");
        assert!(matches!(ann.geometry, AnnotationGeometry::Leader(ref v) if v.len() == 2));
    }

    #[test]
    fn build_refuses_missing_points_or_text() {
        let mut tool = AnnotationTool::new(AnnotationToolKind::Leader);
        tool.push_point(p(0.0, 0.0)).unwrap();
        assert!(matches!(build(&tool), Err(CadError::InvalidInput(_))));
        tool.push_point(p(1.0, 1.0)).unwrap();
        // Still missing text.
        assert!(matches!(build(&tool), Err(CadError::InvalidInput(_))));
        tool.set_text("  ").unwrap();
        // Whitespace-only is not a real payload.
        assert!(matches!(build(&tool), Err(CadError::InvalidInput(_))));
    }

    #[test]
    fn each_kind_builds_its_geometry() {
        let mut text = AnnotationTool::new(AnnotationToolKind::Text);
        text.push_point(p(2.0, 3.0)).unwrap();
        text.set_text("t").unwrap();
        assert!(matches!(
            build(&text).unwrap().geometry,
            AnnotationGeometry::Text(_)
        ));

        let mut rect = AnnotationTool::new(AnnotationToolKind::Rectangle);
        rect.push_point(p(0.0, 0.0)).unwrap();
        rect.push_point(p(4.0, 3.0)).unwrap();
        assert!(
            matches!(build(&rect).unwrap().geometry, AnnotationGeometry::Rectangle(pair) if pair == [p(0.0,0.0), p(4.0,3.0)])
        );

        let mut ellipse = AnnotationTool::new(AnnotationToolKind::Ellipse);
        ellipse.push_point(p(0.0, 0.0)).unwrap();
        ellipse.push_point(p(2.0, 1.0)).unwrap();
        match build(&ellipse).unwrap().geometry {
            AnnotationGeometry::Ellipse {
                center,
                axis_u,
                axis_v,
            } => {
                assert_eq!(center, p(0.0, 0.0));
                assert_eq!(axis_u, p(2.0, 0.0));
                assert_eq!(axis_v, p(0.0, 1.0));
            }
            other => panic!("expected ellipse, got {other:?}"),
        }

        for kind in [AnnotationToolKind::Freehand, AnnotationToolKind::Cloud] {
            let mut tool = AnnotationTool::new(kind);
            tool.push_point(p(0.0, 0.0)).unwrap();
            tool.push_point(p(1.0, 0.0)).unwrap();
            tool.push_point(p(1.0, 1.0)).unwrap();
            let ann = build(&tool).unwrap();
            let ok = match kind {
                AnnotationToolKind::Freehand => {
                    matches!(ann.geometry, AnnotationGeometry::Freehand(ref v) if v.len() == 3)
                }
                AnnotationToolKind::Cloud => {
                    matches!(ann.geometry, AnnotationGeometry::Cloud(ref v) if v.len() == 3)
                }
                _ => false,
            };
            assert!(ok);
            assert_eq!(ann.text, "");
        }
    }

    #[test]
    fn degenerate_ellipse_axes_are_left_for_the_database_to_reject() {
        // The tool never invents a compensating axis; a zero-size ellipse is
        // built as-is and the single write path refuses it.
        let mut tool = AnnotationTool::new(AnnotationToolKind::Ellipse);
        tool.push_point(p(1.0, 1.0)).unwrap();
        tool.push_point(p(1.0, 1.0)).unwrap();
        let ann = build(&tool).unwrap();
        assert!(cad_db::validate_annotation(&ann).is_err());
    }

    #[test]
    fn build_does_not_set_an_anchor() {
        // Anchoring is host/proxy work; the tool must not fabricate a Valid one.
        let mut rect = AnnotationTool::new(AnnotationToolKind::Rectangle);
        rect.push_point(p(0.0, 0.0)).unwrap();
        rect.push_point(p(1.0, 1.0)).unwrap();
        assert!(build(&rect).unwrap().anchor.is_none());
    }

    #[test]
    fn stroke_helpers_do_not_claim_a_commit_precondition() {
        assert!(!stroke_is_closed(&[p(0.0, 0.0), p(1.0, 1.0)]));
        assert!(stroke_is_closed(&[
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(0.0, 0.0)
        ]));
        let square = [
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(0.0, 1.0),
            p(0.0, 0.0),
        ];
        assert!((winding_turns(&square).abs() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn segment_turn_is_signed_and_degenerate_safe() {
        // A left (counter-clockwise) 90-degree turn is +pi/2.
        let left = segment_turn(p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)).unwrap();
        assert!((left - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{left}");
        // A right (clockwise) 90-degree turn is -pi/2.
        let right = segment_turn(p(0.0, 0.0), p(1.0, 0.0), p(1.0, -1.0)).unwrap();
        assert!(
            (right + std::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "{right}"
        );
        // A straight continuation is a zero turn.
        let straight = segment_turn(p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)).unwrap();
        assert!(straight.abs() < 1e-9, "{straight}");
        // A zero-length segment has no defined turn.
        assert!(segment_turn(p(0.0, 0.0), p(0.0, 0.0), p(1.0, 0.0)).is_none());
    }
}
