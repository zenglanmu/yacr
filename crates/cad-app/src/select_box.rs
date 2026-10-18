//! Read-only window/crossing selection over the existing model-space pick
//! catalogue (ADR 0014 B).
//!
//! The command is deliberately shaped like [`crate::select_all`]: it preflights
//! the same visible entity tree, reuses the same conservative AABB broad phase
//! (`drawing_pick_items` + `cad_spatial::pick_world_bounds`) and rewrites only
//! the session selection — never the database, the revision or the history.
//!
//! **Precision is stated per path, never guessed.**
//!
//! * WINDOW (`crossing == false`) accepts an item only when its **conservative**
//!   world AABB is fully inside the rectangle. That is sound (the geometry is
//!   necessarily inside too), but it can *under-select*: a conservative box may
//!   enclose more than the geometry (for example a spline's control hull), so a
//!   shape lying wholly inside the rectangle but with a box poking out is
//!   missed. This is the documented precision limit of ADR 0014 B.7, not a
//!   silent exact result.
//! * CROSSING (`crossing == true`) first rejects items whose AABB misses the
//!   rectangle. Straight segments and straight polylines are then tested with an
//!   exact rectangle clip; arcs, circles and ellipses are chord-tessellated and
//!   marked **approximate**. Meshes, text, shapes, images, masks and compound
//!   children with no exact path fall back to the AABB test and are reported as
//!   such. Splines and bulge polylines have no public tessellation helper
//!   reachable from this crate without adding a Cargo dependency, so they are
//!   AABB-only here (reported, never presented as exact).
//!
//! Paper space and 3D are explicit refusals — this command only maps canvas
//! logical coordinates through the orthographic work plane.

use std::borrow::Cow;

use cad_db::{transform_geometry, DbEntity, DrawingDatabase};
use cad_domain::{
    BlockId, CadError, CadResult, Diagnostic, Point3, SemanticGeometry, SpaceId, Transform3,
};
use cad_representation::{clip_polyline_to_rect, ConicSegment};
use cad_spatial::pick_world_bounds;

use crate::{
    api::apply_selection, drawing_pick_items, Application, Command, CommandOutcome, CommandPayload,
    SessionState, ToolState,
};

/// Which predicate produced a match. Reported in the `select.box.count`
/// diagnostic so an approximate/AABB result is never mistaken for an exact one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchPath {
    /// WINDOW: conservative AABB fully inside.
    Window,
    /// CROSSING: exact segment/polyline rectangle clip.
    Exact,
    /// CROSSING: chord-tessellated curve rectangle clip.
    Approximate,
    /// CROSSING: conservative AABB only.
    Aabb,
}

impl MatchPath {
    fn rank(self) -> u8 {
        match self {
            MatchPath::Exact => 0,
            MatchPath::Approximate => 1,
            MatchPath::Aabb | MatchPath::Window => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    min: (f64, f64),
    max: (f64, f64),
}

impl Rect {
    fn from_corners(a: Point3, b: Point3) -> Self {
        Rect {
            min: (a.x.min(b.x), a.y.min(b.y)),
            max: (a.x.max(b.x), a.y.max(b.y)),
        }
    }

    fn width(self) -> f64 {
        self.max.0 - self.min.0
    }

    fn height(self) -> f64 {
        self.max.1 - self.min.1
    }

    fn center_half(self) -> ([f64; 2], [f64; 2]) {
        (
            [
                (self.min.0 + self.max.0) * 0.5,
                (self.min.1 + self.max.1) * 0.5,
            ],
            [self.width() * 0.5, self.height() * 0.5],
        )
    }

    fn contains_point(self, point: Point3) -> bool {
        point.x >= self.min.0
            && point.x <= self.max.0
            && point.y >= self.min.1
            && point.y <= self.max.1
    }

    /// Whether a conservative world AABB is fully inside (WINDOW).
    fn contains_aabb(self, min: Point3, max: Point3) -> bool {
        min.x >= self.min.0 && max.x <= self.max.0 && min.y >= self.min.1 && max.y <= self.max.1
    }

    /// Whether a conservative world AABB intersects or touches (CROSSING
    /// broad phase).
    fn intersects_aabb(self, min: Point3, max: Point3) -> bool {
        max.x >= self.min.0 && min.x <= self.max.0 && max.y >= self.min.1 && min.y <= self.max.1
    }
}

impl Application {
    pub(crate) fn select_in_box(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::BoxSelect {
            rect_logical,
            crossing,
            mode,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "SelectInBox needs a BoxSelect payload".into(),
            ));
        };
        let (crossing, mode) = (*crossing, *mode);

        if !rect_logical.iter().flatten().all(|value| value.is_finite()) {
            return Err(CadError::InvalidInput(
                "box selection corners must be finite".into(),
            ));
        }

        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.id != command.document {
            return Err(CadError::StaleResult);
        }
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "SelectInBox currently supports model space only".into(),
            ));
        }
        // Mirror `select_at_mode`: a drawing/measuring tool owns the pointer, so a
        // box gesture must not rewrite the selection underneath it.
        if !matches!(session.tool, ToolState::Idle | ToolState::Selecting) {
            return Ok(CommandOutcome::none());
        }

        let viewport = self
            .workspace
            .viewports
            .get(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let size = viewport.logical_size;
        if !size.iter().all(|value| value.is_finite() && *value > 0.0) {
            return Err(CadError::InvalidInput(
                "box selection viewport size must be finite and positive".into(),
            ));
        }
        // The plan mapping is the same one picking uses for a canvas point. A
        // perspective/3D view has no plan mapping: 3D box selection is an
        // explicit NOT IMPLEMENTED refusal.
        let first = viewport
            .screen_to_world(rect_logical[0], size)
            .ok_or_else(|| {
                CadError::Unsupported("SelectInBox requires the orthographic plan view".into())
            })?;
        let second = viewport
            .screen_to_world(rect_logical[1], size)
            .ok_or_else(|| {
                CadError::Unsupported("SelectInBox requires the orthographic plan view".into())
            })?;
        let rect = Rect::from_corners(first, second);
        if !rect.width().is_finite()
            || !rect.height().is_finite()
            || rect.width() <= 0.0
            || rect.height() <= 0.0
        {
            return Err(CadError::InvalidInput(
                "box selection rectangle must be non-degenerate".into(),
            ));
        }

        let database = document.drawing.as_ref();
        // The pick builder silently skips cycles/depth limits; preflight the same
        // visible tree `select_all` refuses on before trusting its catalogue.
        for entity in database.model_space() {
            validate_branch(database, session, entity, &mut Vec::new())?;
        }

        let mut matched: Vec<_> = Vec::new();
        let mut path_counts = [0usize; 3]; // exact, approximate, aabb
        for item in drawing_pick_items(database, command.document) {
            let mut visible = true;
            for id in item
                .source
                .instance
                .0
                .iter()
                .chain(std::iter::once(&item.source.entity))
            {
                let entity = database.entity(*id).ok_or(CadError::StaleResult)?;
                visible &= session.layer_overrides.is_entity_visible(database, entity);
            }
            if !visible {
                continue;
            }
            let Some((bounds_min, bounds_max)) = pick_world_bounds(&item.geometry, &item.transform)
            else {
                // No conservative box: never fabricate an inclusion.
                continue;
            };
            let path = if crossing {
                crossing_path(&item.geometry, &item.transform, rect)
            } else if rect.contains_aabb(bounds_min, bounds_max) {
                // Sound but conservative: see the module comment on under-selection.
                Some(MatchPath::Window)
            } else {
                None
            };
            match path {
                Some(MatchPath::Window) => {
                    matched.push(item.source);
                }
                Some(MatchPath::Exact) => {
                    path_counts[0] += 1;
                    matched.push(item.source);
                }
                Some(MatchPath::Approximate) => {
                    path_counts[1] += 1;
                    matched.push(item.source);
                }
                Some(MatchPath::Aabb) => {
                    path_counts[2] += 1;
                    matched.push(item.source);
                }
                None => {}
            }
        }

        let count = matched.len();
        apply_selection(session, matched, mode);

        // Machine keys only: the shell/localization layer owns the prose. `count`
        // is the number of box matches, not the post-merge selection size.
        let precision = if !crossing {
            "window"
        } else {
            match (path_counts[0] > 0, path_counts[1] > 0, path_counts[2] > 0) {
                (false, false, false) => "none",
                (true, false, false) => "exact",
                (false, true, false) => "approximate",
                (false, false, true) => "aabb",
                _ => "mixed",
            }
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "select.box.count".into(),
                message: format!("count={count};precision={precision}"),
            }],
            measurement: None,
        })
    }
}

/// Exact/approximate CROSSING test for one pick item, in world space.
fn crossing_path(
    geometry: &SemanticGeometry,
    transform: &Transform3,
    rect: Rect,
) -> Option<MatchPath> {
    // Conservative broad phase first: a missed AABB cannot be a hit.
    let (bounds_min, bounds_max) = pick_world_bounds(geometry, transform)?;
    if !rect.intersects_aabb(bounds_min, bounds_max) {
        return None;
    }
    let world = if *transform == Transform3::identity() {
        Cow::Borrowed(geometry)
    } else {
        match transform_geometry(geometry, transform) {
            Ok(world) => Cow::Owned(world),
            // The AABB already includes the transform and intersects; without a
            // transformable geometry we cannot do better, and say so.
            Err(_) => return Some(MatchPath::Aabb),
        }
    };
    classify_world(&world, rect)
}

/// Classify a geometry already in world coordinates against the rectangle.
fn classify_world(geometry: &SemanticGeometry, rect: Rect) -> Option<MatchPath> {
    let (bounds_min, bounds_max) = pick_world_bounds(geometry, &Transform3::identity())?;
    if !rect.intersects_aabb(bounds_min, bounds_max) {
        return None;
    }
    match geometry {
        SemanticGeometry::Line { start, end } => {
            segment_hits(*start, *end, rect).then_some(MatchPath::Exact)
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if points.len() < 2 {
                return None;
            }
            if bulges.iter().any(|bulge| bulge.abs() > 1e-12) {
                // A bulge segment is a circular arc, not the chord; the exact
                // chord test would be a different shape. No public segment-wise
                // bulge tessellator is reachable from this crate without a Cargo
                // dependency, so the conservative AABB bound (reported as `aabb`)
                // is used rather than a silently wrong chord.
                return Some(MatchPath::Aabb);
            }
            let mut polyline = points.clone();
            if *closed {
                polyline.push(points[0]);
            }
            let (center, half) = rect.center_half();
            (!clip_polyline_to_rect(&polyline, center, half).is_empty()).then_some(MatchPath::Exact)
        }
        SemanticGeometry::Arc { .. }
        | SemanticGeometry::Circle { .. }
        | SemanticGeometry::Ellipse { .. } => approximate_curve_path(geometry, rect),
        SemanticGeometry::Point(point) => rect.contains_point(*point).then_some(MatchPath::Exact),
        SemanticGeometry::Compound(children) => children
            .iter()
            .filter_map(|child| classify_world(child, rect))
            .min_by_key(|path| path.rank()),
        // Splines have no rectangle tessellator reachable from this crate
        // without a Cargo dependency (frozen for this change); the honest path
        // is the conservative AABB with an `aabb` report.
        // Mesh/Text/Shape/Image/Mask/Insert/Opaque likewise fall back to the
        // AABB bound already carried by the pick item.
        _ => Some(MatchPath::Aabb),
    }
}

/// Exact rectangle clip of one straight segment.
fn segment_hits(start: Point3, end: Point3, rect: Rect) -> bool {
    if start == end {
        return rect.contains_point(start);
    }
    let (center, half) = rect.center_half();
    !clip_polyline_to_rect(&[start, end], center, half).is_empty()
}

/// Chord-tessellate a conic and clip the resulting polyline.
///
/// Uses the representation layer's `ConicSegment`, which delegates to the same
/// `cad_geometry::tessellate_geometry` the snap/preview code uses. The result is
/// therefore the same chord approximation, and is reported as approximate.
fn approximate_curve_path(geometry: &SemanticGeometry, rect: Rect) -> Option<MatchPath> {
    let Some(points) = conic_points(geometry) else {
        return Some(MatchPath::Aabb);
    };
    if points.len() < 2 {
        return Some(MatchPath::Aabb);
    }
    let (center, half) = rect.center_half();
    if clip_polyline_to_rect(&points, center, half).is_empty() {
        None
    } else {
        Some(MatchPath::Approximate)
    }
}

/// Tessellate a circle/arc/ellipse through the shared representation conic.
fn conic_points(geometry: &SemanticGeometry) -> Option<Vec<Point3>> {
    const TAU: f64 = std::f64::consts::TAU;
    let conic = match geometry {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let (u, v) = plane_axes(*normal);
            ConicSegment {
                center: *center,
                major: scale(u, *radius),
                minor: scale(v, *radius),
                start: 0.0,
                sweep: TAU,
                full: true,
            }
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (u, v) = plane_axes(*normal);
            ConicSegment {
                center: *center,
                major: scale(u, *radius),
                minor: scale(v, *radius),
                start: *start,
                sweep: *sweep,
                full: sweep.abs() >= TAU - 1e-9,
            }
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let major_len = length(*major_axis);
            if !major_len.is_finite() || major_len <= 1e-12 {
                return None;
            }
            let major_unit = scale(*major_axis, 1.0 / major_len);
            let minor_dir = normalize(cross(normalize(*normal), major_unit));
            if length(minor_dir) <= 1e-12 {
                return None;
            }
            ConicSegment {
                center: *center,
                major: *major_axis,
                minor: scale(minor_dir, major_len * ratio.abs()),
                start: *start,
                sweep: *sweep,
                full: sweep.abs() >= TAU - 1e-9,
            }
        }
        _ => return None,
    };
    Some(conic.chord_points())
}

/// The AutoCAD arbitrary-axis frame, matching the conic tessellation's own frame
/// so an arc's start angle denotes the same point here.
fn plane_axes(normal: Point3) -> (Point3, Point3) {
    let n = if length(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        normalize(normal)
    };
    const ONE_64TH: f64 = 1.0 / 64.0;
    let axis = if n.x.abs() < ONE_64TH && n.y.abs() < ONE_64TH {
        cross(
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            n,
        )
    } else {
        cross(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            n,
        )
    };
    let u = if length(axis) < 1e-24 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        normalize(axis)
    };
    (u, normalize(cross(n, u)))
}

fn scale(a: Point3, factor: f64) -> Point3 {
    Point3 {
        x: a.x * factor,
        y: a.y * factor,
        z: a.z * factor,
    }
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn length(a: Point3) -> f64 {
    (a.x * a.x + a.y * a.y + a.z * a.z).sqrt()
}

fn normalize(a: Point3) -> Point3 {
    let len = length(a);
    if len <= 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        scale(a, 1.0 / len)
    }
}

/// Same preflight `select_all` performs: refuse cyclic, over-deep, missing-block
/// or opaque/unsupported trees instead of publishing a partial catalogue.
fn validate_branch(
    database: &DrawingDatabase,
    session: &SessionState,
    entity: &DbEntity,
    stack: &mut Vec<BlockId>,
) -> CadResult<()> {
    crate::select_all::validate_branch(database, session, entity, stack)
}
