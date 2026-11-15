//! The STRETCH crossing-**window** capture modes (ADR 0016 decision E).
//!
//! STRETCH is the one capture whose window is not a fixed point run: AutoCAD
//! accepts the classic two-corner rectangle (the default), a crossing *polygon*
//! built from successive vertices, and a freehand *lasso* whose traced path is
//! the same polygon. All three end in the one planner (`stretch_window`), so the
//! mode only describes **how the ring is collected** — never a second containment
//! rule, and never a second payload: a ring is handed to the command layer as the
//! existing `StretchPolygon` payload, whose even-odd point-in-polygon predicate is
//! the single polygon rule.
//!
//! This module is the pure half: the mode vocabulary (mirrored by the
//! `stretchWindow` tool parameter's closed set) and the ring validation the
//! capture applies before it accepts the vertices. The state machine lives in
//! [`super::DrawTool`].

use cad_domain::{CadError, CadResult, Point3};

use super::parameter::{STRETCH_WINDOW_LASSO, STRETCH_WINDOW_POLYGON, STRETCH_WINDOW_RECT};

/// The minimum number of boundary vertices a crossing polygon/lasso needs.
///
/// Two vertices enclose nothing (they are a line), and the command layer refuses
/// the same count in its own words; the capture refuses it earlier so the user
/// never reaches a "close the ring" step that cannot succeed.
pub const STRETCH_MIN_WINDOW_VERTICES: usize = 3;

/// How a STRETCH capture collects its crossing window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StretchWindowMode {
    /// The classic two opposite corners (AutoCAD's default, and the only shape
    /// this build had before the polygon/lasso wiring): four captured points —
    /// two window corners, the base point and the displacement point.
    #[default]
    Rect,
    /// A crossing polygon: every click adds a boundary vertex; the ring closes on
    /// a click on its first vertex or on `CONFIRM`, then the base point and the
    /// displacement point follow as clicks.
    Polygon,
    /// A freehand lasso: one drag traces the ring and its release closes it (the
    /// host samples the pointer path), then the base and displacement points
    /// follow as clicks. A click without a drag cannot define a lasso and is
    /// refused by name.
    Lasso,
}

impl StretchWindowMode {
    /// The mode a `stretchWindow` parameter value selects, or `None` for a value
    /// outside the documented closed set.
    ///
    /// The parameter channel validates on write, so this defensive read can only
    /// see a corrupt store; `None` keeps the capture on the rectangle default
    /// rather than inventing a fourth shape.
    pub fn from_parameter(value: f64) -> Option<Self> {
        if value == STRETCH_WINDOW_RECT {
            Some(Self::Rect)
        } else if value == STRETCH_WINDOW_POLYGON {
            Some(Self::Polygon)
        } else if value == STRETCH_WINDOW_LASSO {
            Some(Self::Lasso)
        } else {
            None
        }
    }

    /// Whether the mode collects a free-form ring before its displacement points.
    pub fn is_ring(self) -> bool {
        !matches!(self, Self::Rect)
    }

    /// Stable machine word used by the capture's refusal messages.
    pub fn key(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Polygon => "polygon",
            Self::Lasso => "lasso",
        }
    }
}

/// The crossing-window phase an **active** STRETCH capture is collecting.
///
/// The host reads this to route a pointer gesture — never a command, never a
/// payload: a lasso's ring arrives as a drag, a polygon's as clicks, and the
/// rectangle window as neither. `None` (no step) means the capture is past its
/// window (its displacement points are ordinary click picks) or no STRETCH
/// capture is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchWindowStep {
    /// The polygon ring is open: another click adds a boundary vertex, and
    /// `CONFIRM` (or a click on the first vertex) closes it.
    PolygonVertex,
    /// The lasso is waiting for its freehand drag: one press-drag-release traces
    /// the whole ring.
    LassoTrace,
}

/// Validate a free-form crossing ring before the capture accepts it.///
/// A ring needs at least [`STRETCH_MIN_WINDOW_VERTICES`] finite vertices and a
/// non-zero XY area: a collinear path (or a drag that never moved) encloses
/// nothing, so it is refused **by name** here instead of being handed to the
/// command layer as a window that would silently select nothing.
///
/// Self-intersection is deliberately **not** refused: the container is the
/// command layer's even-odd point-in-polygon predicate, which is well defined for
/// a crossing (figure-eight) ring — that is exactly what a real freehand lasso
/// produces, and refusing it would make the lasso unusable. The containment rule
/// is the documented one (`StretchWindow::Polygon`), not a new one.
pub fn validate_window_ring(vertices: &[Point3], mode: StretchWindowMode) -> CadResult<()> {
    if vertices.len() < STRETCH_MIN_WINDOW_VERTICES {
        return Err(CadError::InvalidInput(format!(
            "stretch.window.too_few_vertices: a {} crossing window needs at least {} boundary \
             vertices, got {}",
            mode.key(),
            STRETCH_MIN_WINDOW_VERTICES,
            vertices.len()
        )));
    }
    if vertices
        .iter()
        .any(|vertex| !vertex.x.is_finite() || !vertex.y.is_finite() || !vertex.z.is_finite())
    {
        return Err(CadError::InvalidInput(
            "stretch.window.non_finite: crossing window vertices must be finite".into(),
        ));
    }
    let count = vertices.len();
    let mut area = 0.0;
    let mut previous = count - 1;
    for index in 0..count {
        let (x0, y0) = (vertices[previous].x, vertices[previous].y);
        let (x1, y1) = (vertices[index].x, vertices[index].y);
        area += x0 * y1 - x1 * y0;
        previous = index;
    }
    if !area.is_finite() || area == 0.0 {
        return Err(CadError::InvalidInput(format!(
            "stretch.window.degenerate_ring: the {} crossing window encloses no area (its \
             vertices are collinear or repeat), so it is refused instead of selecting nothing",
            mode.key()
        )));
    }
    Ok(())
}

impl super::DrawTool {
    /// Select how this STRETCH capture collects its crossing window.
    ///
    /// The session applies the live `stretchWindow` parameter when the capture
    /// starts (and re-plans it only while no vertex has been captured yet), so the
    /// mode is fixed before the first click and a ring can never be silently
    /// reinterpreted as a rectangle. [`StretchWindowMode::Rect`] is the default and
    /// restores the four-point rectangle capture exactly.
    pub fn set_window_mode(&mut self, mode: StretchWindowMode) {
        self.window_mode = mode;
    }

    /// The crossing-window capture mode in force.
    pub fn window_mode(&self) -> StretchWindowMode {
        self.window_mode
    }

    /// Whether the capture is still collecting its free-form window **ring**.
    ///
    /// `true` only for a polygon/lasso STRETCH capture whose ring has not been
    /// closed yet: every click there is another boundary vertex.
    pub fn window_open(&self) -> bool {
        self.window_mode.is_ring() && !self.ring_closed
    }

    /// The captured crossing-window ring, in click/trace order.
    pub fn window_ring(&self) -> &[Point3] {
        &self.window_ring
    }

    /// The first captured ring vertex, or `None` while the ring is empty.
    ///
    /// The host reads it to apply AutoCAD's "click the first vertex to close the
    /// ring" rule without inventing a second ring model.
    pub fn first_window_vertex(&self) -> Option<Point3> {
        self.window_ring.first().copied()
    }

    /// Close the free-form window ring so the capture moves on to the base and
    /// displacement points (`CONFIRM` at a vertex prompt, or a click on the first
    /// vertex).
    ///
    /// A ring with fewer than [`STRETCH_MIN_WINDOW_VERTICES`] vertices cannot
    /// enclose anything and is refused **by name**; nothing is dropped, so the
    /// user can keep clicking. A degenerate (zero-area) ring is refused by the
    /// same rule the command layer applies, so a collinear run never becomes a
    /// window that selects nothing.
    pub fn close_window_ring(&mut self) -> CadResult<()> {
        if !self.window_open() {
            return Err(CadError::InvalidInput(format!(
                "stretch.window.not_open: draw kind '{}' has no open crossing-window ring to close",
                self.kind.key()
            )));
        }
        validate_window_ring(&self.window_ring, self.window_mode)?;
        self.ring_closed = true;
        Ok(())
    }

    /// Record a **complete** free-form ring captured outside the click path (the
    /// host's freehand lasso drag) and close it.
    ///
    /// The ring is validated exactly like the clicked one
    /// ([`validate_window_ring`]), so a lasso and a polygon can never disagree
    /// about what a usable window is. Only a polygon/lasso STRETCH capture with an
    /// open ring accepts one; every other state refuses by name and stores
    /// nothing.
    pub fn set_window_ring(&mut self, ring: Vec<Point3>) -> CadResult<()> {
        if !self.window_open() {
            return Err(CadError::InvalidInput(format!(
                "stretch.window.not_open: draw kind '{}' is not waiting for a crossing-window \
                 ring",
                self.kind.key()
            )));
        }
        validate_window_ring(&ring, self.window_mode)?;
        self.window_ring = ring;
        self.ring_closed = true;
        Ok(())
    }

    /// Append one crossing-window ring vertex (STRETCH polygon).
    ///
    /// Finite-only and bounded by the shared open-capture cap, exactly like an
    /// open-ended point run: a ring that grew without bound would be a runaway
    /// capture, so the `(MAX + 1)`-th vertex is refused by name instead of being
    /// dropped silently. A **lasso** ring is traced by the host, never by clicks,
    /// so a click there is refused by name rather than starting a vertex run the
    /// user did not ask for.
    pub(super) fn push_window_vertex(&mut self, point: Point3) -> CadResult<()> {
        if self.window_mode == StretchWindowMode::Lasso {
            return Err(CadError::InvalidInput(
                "stretch.window.lasso_needs_drag: a crossing lasso is traced with one freehand \
                 drag; a click cannot define it (nothing was captured)"
                    .into(),
            ));
        }
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput(
                "stretch.window.non_finite: crossing window vertices must be finite".into(),
            ));
        }
        if self.window_ring.len() >= super::MAX_OPEN_CAPTURE_POINTS {
            return Err(CadError::InvalidInput(format!(
                "stretch.window.too_many_vertices: a crossing polygon takes at most {} vertices",
                super::MAX_OPEN_CAPTURE_POINTS
            )));
        }
        self.window_ring.push(point);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn the_parameter_closed_set_maps_onto_the_three_modes() {
        assert_eq!(
            StretchWindowMode::from_parameter(0.0),
            Some(StretchWindowMode::Rect)
        );
        assert_eq!(
            StretchWindowMode::from_parameter(1.0),
            Some(StretchWindowMode::Polygon)
        );
        assert_eq!(
            StretchWindowMode::from_parameter(2.0),
            Some(StretchWindowMode::Lasso)
        );
        // A value the channel would have refused never invents a fourth shape.
        assert_eq!(StretchWindowMode::from_parameter(3.0), None);
        assert_eq!(StretchWindowMode::from_parameter(f64::NAN), None);
        assert!(!StretchWindowMode::Rect.is_ring());
        assert!(StretchWindowMode::Polygon.is_ring());
        assert!(StretchWindowMode::Lasso.is_ring());
    }

    #[test]
    fn a_real_ring_is_accepted_and_a_degenerate_one_is_refused_by_name() {
        let square = [p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)];
        validate_window_ring(&square, StretchWindowMode::Polygon).unwrap();
        // Reversed winding is the same ring (the command's predicate is
        // winding-independent), so the capture accepts it too.
        let mut reversed = square;
        reversed.reverse();
        validate_window_ring(&reversed, StretchWindowMode::Lasso).unwrap();

        // Two vertices are a line, not a ring.
        let error = validate_window_ring(&square[..2], StretchWindowMode::Polygon).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("stretch.window.too_few_vertices"),
            "{error}"
        );
        // Collinear vertices enclose nothing.
        let collinear = [p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)];
        let error = validate_window_ring(&collinear, StretchWindowMode::Lasso).unwrap_err();
        assert!(
            error.to_string().contains("stretch.window.degenerate_ring"),
            "{error}"
        );
        // A non-finite vertex is refused by name, never stored.
        let mut broken = square;
        broken[1].y = f64::NAN;
        let error = validate_window_ring(&broken, StretchWindowMode::Polygon).unwrap_err();
        assert!(
            error.to_string().contains("stretch.window.non_finite"),
            "{error}"
        );
    }

    #[test]
    fn a_self_intersecting_ring_is_the_documented_even_odd_shape_not_a_refusal() {
        // A figure-eight: the freehand path a lasso routinely produces. Its even-odd
        // containment is well defined, so an asymmetric crossing ring is accepted
        // (refusing it would make the lasso unusable) and the command layer owns
        // the containment predicate.
        let crossed = [
            p(0.0, 0.0),
            p(4.0, 4.0),
            p(4.0, 0.0),
            p(0.0, 2.0),
            p(0.0, 0.0),
        ];
        validate_window_ring(&crossed, StretchWindowMode::Lasso).unwrap();

        // A **symmetric** figure-eight cancels to a zero signed area, so it is
        // refused like any other zero-area ring — the named refusal is the area
        // rule, never a silent "selects nothing".
        let symmetric = [
            p(0.0, 0.0),
            p(2.0, 2.0),
            p(2.0, 0.0),
            p(0.0, 2.0),
            p(0.0, 0.0),
        ];
        let error = validate_window_ring(&symmetric, StretchWindowMode::Lasso).unwrap_err();
        assert!(
            error.to_string().contains("stretch.window.degenerate_ring"),
            "{error}"
        );
    }
}
