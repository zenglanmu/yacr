//! `REVCLOUD` — AutoCAD 2026's revision-cloud command.
//!
//! ## Implemented options
//!
//! * **Object** — converts the one selected `CIRCLE`, `ELLIPSE`, `LWPOLYLINE`
//!   or `SPLINE` into a cloud. The source is rewritten in place (same id) for a
//!   polyline; an analytic curve is tessellated, resampled and stored as the
//!   cloud polyline.
//! * **Rectangular** — two opposite corners, optionally reversing the arc side.
//! * **Polygonal** — a vertex run; `closed` closes the ring.
//! * **Freehand** — a point run; `closed` closes the ring.
//! * **Arc Length** — the approximate **chord** length of every arc. The value
//!   is resolved in AutoCAD's order: the command's own `Arc Length` value (the
//!   payload field) wins; otherwise the session arc-length token
//!   (`DraftingExtras::revcloud_arc_length`, the same token
//!   `REVCLOUDPROPERTIES` reports, default 15); only when that token is unusable
//!   (non-finite or `<= 0`) does the declared surrogate apply.
//! * **Style** — `Normal` is the stored style (a named no-op); `Calligraphy` is
//!   a named refusal because the model has no style field.
//! * **Modify** — inserts the supplied points on the cloud path and opens the
//!   ring at the arc nearest the pick side.
//!
//! ## Arc rule (declared)
//!
//! The source path is resampled at a constant distance and every resulting
//! segment carries one arc of fixed **bulge magnitude** [`REVCLOUD_BULGE`]
//! (`0.5`, sweep `4·atan(0.5) ≈ 106.26°`). The sampled segment length is the
//! approximate chord length, so `Arc Length` is honoured at the chord level, not
//! at an exact arc-length integral. The sign follows the ring winding (positive
//! on a counter-clockwise closed ring = bows outward) and is negated by the
//! documented `reverse` flag.
//!
//! When neither the payload nor the session token supplies a usable length,
//! [`DEFAULT_ARC_LENGTH_FRACTION`] (5 %) of the source path's bounding-box
//! diagonal is the declared fallback. This build has no viewport in the command
//! layer, so the path diagonal is the deterministic surrogate for AutoCAD's
//! view-diagonal rule. Every diagnostic names which of the three sources was
//! used, so a cloud never silently mixes conventions.
//!
//! ## Result and write discipline
//!
//! A closed input produces a closed cloud; a freehand/polygonal path that is not
//! closed produces a closed ring only when `closed` is set, otherwise an open
//! cloud. Every refusal happens before a transaction. A successful conversion is
//! **one** transaction / **one** undo step; Object/Modify update the source id,
//! while the three point-run modes insert one entity on the active layer.
//!
//! Refusals are stable ASCII codes: `revcloud.payload`, `revcloud.selection`,
//! `revcloud.not_addressable`, `revcloud.target_missing`,
//! `revcloud.not_polyline`, `revcloud.not_convertible`, `revcloud.open_polyline`,
//! `revcloud.too_few_vertices`, `revcloud.bulged_segment`,
//! `revcloud.invalid_arc_length`, `revcloud.degenerate`, `revcloud.not_planar`,
//! `revcloud.too_many_vertices`, `revcloud.modify_open` and
//! `revcloud.style_unsupported`.
//!
//! ## Calligraphy (declared capability gap)
//!
//! AutoCAD's `Calligraphy` style is a **brush-stroke rendering** of the cloud
//! arcs: the stroke width/shape varies along each arc. This build's cloud is a
//! `SemanticGeometry::Polyline` whose only per-segment shape parameter is the
//! bulge (one circular arc), so there is no field that could carry a
//! calligraphic stroke. Rather than fake the style (e.g. by alternating bulge
//! magnitudes, which would be a different geometry, not the vendor style),
//! `REVCLOUD STYLE CALLIGRAPHY` stays an `Unsupported` refusal under
//! `revcloud.style_unsupported` and writes nothing.

use super::*;
use crate::command_payload::{
    RevCloudStyle, RevisionCloudMode, RevisionCloudOptions, UtilityPayload,
};
use cad_geometry::{is_xy_parallel, signed_area, tessellate_geometry, TessellationParams};

/// The bulge magnitude every generated arc carries. AutoCAD's exact arc-length
/// construction is not reproduced; the declared rule is one 106.26° arc per
/// constant-chord segment.
pub const REVCLOUD_BULGE: f64 = 0.5;

/// The first-cloud default chord as a fraction of the source path's bounding-box
/// diagonal (the deterministic surrogate for AutoCAD's view diagonal).
pub const DEFAULT_ARC_LENGTH_FRACTION: f64 = 0.05;

/// A segment whose bulge exceeds this magnitude is already an arc.
const REVCLOUD_STRAIGHT_TOLERANCE: f64 = 1e-9;

/// The smallest polygon area (world units squared) that is not degenerate.
const REVCLOUD_MIN_AREA: f64 = 1e-12;

/// The hard vertex budget of one generated cloud.
const REVCLOUD_MAX_VERTICES: usize = 4096;

/// A `revcloud.<code>: <detail>` refusal string (ASCII only).
fn refusal(code: &str, detail: impl std::fmt::Display) -> String {
    format!("revcloud.{code}: {detail}")
}

/// A named, non-capability refusal (bad input for a supported shape).
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(refusal(code, detail))
}

/// A named capability-gap refusal (nothing is written).
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(refusal(code, detail))
}

/// The `REVCLOUD` options carried by this command, or the documented default
/// Object mode for the bare selection form.
fn options(command: &Command) -> CadResult<RevisionCloudOptions> {
    match utility_payload(command) {
        Some(UtilityPayload::RevisionCloud(options)) => Ok(options.clone()),
        Some(_) => Err(invalid(
            "payload",
            "REVCLOUD does not accept another utility command's payload",
        )),
        None => match &command.payload {
            CommandPayload::None => Ok(RevisionCloudOptions {
                mode: RevisionCloudMode::Object,
                arc_length: None,
            }),
            _ => Err(invalid(
                "payload",
                "REVCLOUD takes no payload (Object mode) or a RevisionCloud utility payload",
            )),
        },
    }
}

/// Where the chord length of one cloud came from (echoed by the diagnostics).
///
/// The precedence is AutoCAD's: the command's own `Arc Length` prompt (the
/// payload field) wins, then the stored `REVCLOUDPROPERTIES` arc-length token
/// (`DraftingExtras::revcloud_arc_length`, the session value the
/// `REVCLOUDPROPERTIES` report reads), and only when the token is unusable
/// (non-finite or `<= 0`) does the declared deterministic surrogate apply —
/// [`DEFAULT_ARC_LENGTH_FRACTION`] of the source path's bounding-box diagonal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChordSource {
    /// The `Arc Length` value carried by this command's payload.
    Explicit,
    /// The session `REVCLOUDPROPERTIES` arc-length token.
    SessionToken,
    /// The declared 5 %-of-diagonal surrogate (the token was unusable).
    PathDiagonal,
}

impl ChordSource {
    /// The locale-neutral label the diagnostics echo.
    fn label(self) -> &'static str {
        match self {
            Self::Explicit => "explicit Arc Length",
            Self::SessionToken => "session REVCLOUDPROPERTIES arc-length token",
            Self::PathDiagonal => "5% of the path diagonal (declared surrogate)",
        }
    }
}

/// Validate the explicit chord, then the session token, then the deterministic
/// fallback ([`ChordSource`] for the precedence).
fn chord_length(
    explicit: Option<f64>,
    session_token: f64,
    diagonal: f64,
) -> CadResult<(f64, ChordSource)> {
    if let Some(value) = explicit {
        if !value.is_finite() || value <= 0.0 {
            return Err(invalid(
                "invalid_arc_length",
                format!("Arc Length must be finite and strictly positive, got {value}"),
            ));
        }
        return Ok((value, ChordSource::Explicit));
    }
    // The stored token is the documented default for a new cloud. A corrupt
    // token (non-finite / non-positive) is not a refusal of the whole command:
    // the declared diagonal surrogate takes over and the diagnostic names it.
    if session_token.is_finite() && session_token > 0.0 {
        return Ok((session_token, ChordSource::SessionToken));
    }
    if !diagonal.is_finite() || diagonal <= 0.0 {
        return Err(invalid(
            "degenerate",
            "the source path has no positive bounding-box diagonal for the default Arc Length",
        ));
    }
    Ok((
        (diagonal * DEFAULT_ARC_LENGTH_FRACTION).max(1e-9),
        ChordSource::PathDiagonal,
    ))
}

/// Validate the explicit `Arc Length` value alone (the `Style` form stores no
/// geometry, so it only needs the payload validation the build forms share).
fn validate_explicit_chord(explicit: Option<f64>) -> CadResult<()> {
    if let Some(value) = explicit {
        if !value.is_finite() || value <= 0.0 {
            return Err(invalid(
                "invalid_arc_length",
                format!("Arc Length must be finite and strictly positive, got {value}"),
            ));
        }
    }
    Ok(())
}

/// The world-space diagonal of the path's axis-aligned bounding box.
fn bounding_diagonal(points: &[Point3]) -> f64 {
    let Some(first) = points.first() else {
        return 0.0;
    };
    let (mut min, mut max) = (*first, *first);
    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    (max.x - min.x).hypot(max.y - min.y).hypot(max.z - min.z)
}

/// Drop a repeated closing point from a sampled ring.
fn drop_closing_duplicate(points: &mut Vec<Point3>) {
    if points.len() > 1 {
        let first = points[0];
        let last = *points.last().expect("non-empty");
        let tolerance = 1e-9 * (1.0 + first.x.abs().max(first.y.abs()).max(first.z.abs()));
        if (first.x - last.x)
            .hypot(first.y - last.y)
            .hypot(first.z - last.z)
            <= tolerance
        {
            points.pop();
        }
    }
}

/// Remove consecutive duplicate vertices.
fn dedup_consecutive(points: &mut Vec<Point3>) {
    let mut write = 1;
    for read in 1..points.len() {
        let previous = points[write - 1];
        let current = points[read];
        let tolerance = 1e-9 * (1.0 + previous.x.abs().max(previous.y.abs()).max(previous.z.abs()));
        if (previous.x - current.x)
            .hypot(previous.y - current.y)
            .hypot(previous.z - current.z)
            > tolerance
        {
            points[write] = current;
            write += 1;
        }
    }
    points.truncate(write);
}

/// One point at `distance` along the path.
fn sample_at(points: &[Point3], cumulative: &[f64], closed: bool, distance: f64) -> Point3 {
    let count = points.len();
    let segments = if closed {
        count
    } else {
        count.saturating_sub(1)
    };
    if segments == 0 {
        return points[0];
    }
    let distance = distance.max(0.0).min(cumulative[segments]);
    let mut index = 0;
    while index + 1 < segments && cumulative[index + 1] < distance {
        index += 1;
    }
    let a = points[index];
    let b = points[(index + 1) % count];
    let span = cumulative[index + 1] - cumulative[index];
    let t = if span > 0.0 {
        ((distance - cumulative[index]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Point3 {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        z: a.z + (b.z - a.z) * t,
    }
}

/// Resample a path at a constant chord and author one outward arc per segment.
fn resample_cloud(
    points: Vec<Point3>,
    closed: bool,
    chord: f64,
    reverse: bool,
) -> CadResult<SemanticGeometry> {
    let mut points = points;
    dedup_consecutive(&mut points);
    if closed {
        drop_closing_duplicate(&mut points);
    }
    let minimum = if closed { 3 } else { 2 };
    if points.len() < minimum {
        return Err(invalid(
            "too_few_vertices",
            format!(
                "the path needs at least {minimum} distinct vertices, got {}",
                points.len()
            ),
        ));
    }
    if !points.iter().all(|point| is_finite_point(*point)) {
        return Err(invalid("degenerate", "the path has a non-finite vertex"));
    }
    if !is_xy_parallel(&points) {
        return Err(invalid(
            "not_planar",
            "the path vertices are not coplanar on one +Z plane",
        ));
    }
    let count = points.len();
    let segments = if closed { count } else { count - 1 };
    let mut cumulative = Vec::with_capacity(segments + 1);
    cumulative.push(0.0);
    for index in 0..segments {
        let a = points[index];
        let b = points[(index + 1) % count];
        cumulative.push(cumulative[index] + (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z));
    }
    let total = cumulative[segments];
    if !total.is_finite() || total <= 1e-12 {
        return Err(invalid("degenerate", "the path has no length"));
    }
    let segment_count = (total / chord).ceil();
    if !segment_count.is_finite() || segment_count > REVCLOUD_MAX_VERTICES as f64 {
        return Err(invalid(
            "too_many_vertices",
            format!(
                "Arc Length {chord} would generate more than {REVCLOUD_MAX_VERTICES} arcs; use a \
                 larger Arc Length"
            ),
        ));
    }
    let segments_out = (segment_count as usize).max(minimum);
    let step = total / segments_out as f64;
    let out_count = if closed {
        segments_out
    } else {
        segments_out + 1
    };
    let sampled: Vec<Point3> = (0..out_count)
        .map(|index| sample_at(&points, &cumulative, closed, index as f64 * step))
        .collect();
    let sign = if closed {
        let area = signed_area(&sampled);
        if !area.is_finite() || area.abs() <= REVCLOUD_MIN_AREA {
            return Err(invalid(
                "degenerate",
                format!("the closed path encloses no area (signed area {area})"),
            ));
        }
        if area > 0.0 {
            1.0
        } else {
            -1.0
        }
    } else {
        1.0
    };
    let sign = if reverse { -sign } else { sign };
    Ok(SemanticGeometry::Polyline {
        bulges: vec![sign * REVCLOUD_BULGE; sampled.len()],
        points: sampled,
        closed,
    })
}

/// The tessellation budget for one analytic source (the result is resampled
/// again, so this only needs to preserve the shape).
fn source_params(chord: f64) -> TessellationParams {
    TessellationParams {
        tolerance: (chord * 0.25).max(1e-9),
        max_segments: 2048,
        min_segments: 32,
    }
}

/// Flatten one selected path entity to a point run and a closed flag.
fn flatten_source(entity: &DbEntity, chord: f64) -> CadResult<(Vec<Point3>, bool)> {
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            let straight = bulges
                .iter()
                .all(|bulge| bulge.abs() <= REVCLOUD_STRAIGHT_TOLERANCE);
            let mut points = if straight {
                points.clone()
            } else {
                tessellate_geometry(&entity.geometry, source_params(chord))
            };
            if *closed {
                drop_closing_duplicate(&mut points);
            }
            Ok((points, *closed))
        }
        SemanticGeometry::Circle { .. } => {
            let mut points = tessellate_geometry(&entity.geometry, source_params(chord));
            drop_closing_duplicate(&mut points);
            Ok((points, true))
        }
        SemanticGeometry::Ellipse { sweep, .. } => {
            let closed = sweep.abs() >= std::f64::consts::TAU - 1e-9;
            let mut points = tessellate_geometry(&entity.geometry, source_params(chord));
            if closed {
                drop_closing_duplicate(&mut points);
            }
            Ok((points, closed))
        }
        SemanticGeometry::Spline { .. } => {
            let mut points = tessellate_geometry(&entity.geometry, source_params(chord));
            let closed = points.len() > 2 && {
                let first = points[0];
                let last = *points.last().expect("non-empty");
                (first.x - last.x)
                    .hypot(first.y - last.y)
                    .hypot(first.z - last.z)
                    <= 1e-9
            };
            if closed {
                drop_closing_duplicate(&mut points);
            }
            Ok((points, closed))
        }
        SemanticGeometry::Line { .. } => Err(unsupported(
            "not_convertible",
            "REVCLOUD Object mode converts CIRCLE, ELLIPSE, LWPOLYLINE and SPLINE; a LINE has \
             no closed region to cloud",
        )),
        other => Err(unsupported(
            "not_convertible",
            format!(
                "REVCLOUD Object mode converts CIRCLE, ELLIPSE, LWPOLYLINE and SPLINE; {} is not \
                 supported",
                class_name(other)
            ),
        )),
    }
}

/// The approximate bounding-box diagonal of one source entity, used to derive
/// the deterministic default chord before the source is flattened.
fn source_diagonal(entity: &DbEntity) -> f64 {
    match &entity.geometry {
        SemanticGeometry::Polyline { points, .. } => bounding_diagonal(points),
        SemanticGeometry::Circle { radius, .. } => {
            let diameter = radius.abs() * 2.0;
            diameter.hypot(diameter)
        }
        SemanticGeometry::Ellipse {
            major_axis, ratio, ..
        } => {
            let major = (major_axis.x * major_axis.x
                + major_axis.y * major_axis.y
                + major_axis.z * major_axis.z)
                .sqrt();
            let minor = major * ratio.abs();
            (major * 2.0).hypot(minor * 2.0)
        }
        SemanticGeometry::Spline { control_points, .. } => bounding_diagonal(control_points),
        _ => 0.0,
    }
}

/// Reject a non-finite / degenerate rectangle or point run before sampling.
fn validate_build_points(points: &[Point3]) -> CadResult<()> {
    if points.iter().any(|point| !is_finite_point(*point)) {
        return Err(invalid("degenerate", "a construction point is not finite"));
    }
    Ok(())
}

/// Project `point` onto segment `a`–`b`, returning the projected point and its
/// distance from the input.
fn project_on_segment(a: Point3, b: Point3, point: Point3) -> (Point3, f64) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_sq = dx * dx + dy * dy;
    let t = if length_sq > 0.0 {
        (((point.x - a.x) * dx + (point.y - a.y) * dy) / length_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let projected = Point3 {
        x: a.x + dx * t,
        y: a.y + dy * t,
        z: a.z + (b.z - a.z) * t,
    };
    (
        projected,
        (projected.x - point.x)
            .hypot(projected.y - point.y)
            .hypot(projected.z - point.z),
    )
}

/// Insert every `add` point into the path at its nearest segment.
fn insert_add_points(path: &mut Vec<Point3>, add_points: &[Point3]) {
    for add in add_points {
        if path.len() < 2 {
            path.push(*add);
            continue;
        }
        let segments = path.len();
        let mut best = (0usize, f64::INFINITY, *add);
        for index in 0..segments {
            let (projected, distance) =
                project_on_segment(path[index], path[(index + 1) % segments], *add);
            if distance < best.1 {
                best = (index, distance, projected);
            }
        }
        path.insert(best.0 + 1, best.2);
    }
    dedup_consecutive(path);
}

/// Open a closed cloud by omitting the segment whose chord midpoint is nearest
/// `erase_side`.
fn erase_nearest_segment(
    points: &[Point3],
    bulges: &[f64],
    erase_side: Point3,
) -> CadResult<SemanticGeometry> {
    let count = points.len();
    if count < 4 {
        return Err(invalid(
            "degenerate",
            "Modify needs at least four cloud vertices to erase one segment",
        ));
    }
    let mut best = (0usize, f64::INFINITY);
    for index in 0..count {
        let a = points[index];
        let b = points[(index + 1) % count];
        let mid = Point3 {
            x: (a.x + b.x) * 0.5,
            y: (a.y + b.y) * 0.5,
            z: (a.z + b.z) * 0.5,
        };
        let distance = (mid.x - erase_side.x)
            .hypot(mid.y - erase_side.y)
            .hypot(mid.z - erase_side.z);
        if distance < best.1 {
            best = (index, distance);
        }
    }
    let erased = best.0;
    let mut out_points = Vec::with_capacity(count);
    let mut out_bulges = Vec::with_capacity(count - 1);
    for step in 0..count {
        out_points.push(points[(erased + 1 + step) % count]);
        if step + 1 < count {
            out_bulges.push(bulges[(erased + 1 + step) % count]);
        }
    }
    // One bulge per vertex is the storage convention; the final entry is
    // unused for an open polyline (there is no closing segment).
    out_bulges.push(0.0);
    Ok(SemanticGeometry::Polyline {
        points: out_points,
        bulges: out_bulges,
        closed: false,
    })
}

/// Replace one entity's geometry in one transaction / one undo step.
fn commit_updated_cloud(
    app: &mut Application,
    document_id: DocumentId,
    entity: DbEntity,
    geometry: SemanticGeometry,
    label: &str,
    message: String,
) -> CadResult<CommandOutcome> {
    let document = app
        .workspace
        .documents
        .get_mut(&document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let mut candidate = document.drawing.as_ref().clone();
    let entity_id = entity.id;
    let mut after = entity.clone();
    after.geometry = geometry;
    let transaction = next_transaction_id();
    let change_set = {
        let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
        staged.update_entity(after)?;
        staged.commit()?
    };
    let stored = candidate
        .entity(entity_id)
        .cloned()
        .ok_or_else(|| CadError::Invariant(format!("{label}: the entity vanished after commit")))?;
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.to_string(),
            patches: vec![drawing_patch(entity_id, Some(entity), Some(stored))],
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok(CommandOutcome {
        objects: vec![ObjectId(entity_id.0)],
        changes: Some(Arc::new(change_set)),
        diagnostics: vec![Diagnostic {
            object: Some(ObjectId(entity_id.0)),
            code: format!("{label}.applied"),
            message,
        }],
        measurement: None,
        point_report: None,
    })
}

impl Application {
    /// `REVCLOUD`: convert, build or modify one revision cloud.
    pub(crate) fn revision_cloud(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let options = options(command)?;
        // The stored `REVCLOUDPROPERTIES` token is read once here and threaded to
        // every build path; the field's own `Arc Length` overrides it.
        let session_token = session.drafting_extras.revcloud_arc_length();
        match &options.mode {
            RevisionCloudMode::Style(style) => {
                validate_explicit_chord(options.arc_length)?;
                match style {
                    RevCloudStyle::Normal => Ok(CommandOutcome {
                        objects: Vec::new(),
                        changes: None,
                        diagnostics: vec![Diagnostic {
                            object: None,
                            code: "revcloud.style".into(),
                            message: "REVCLOUD Style Normal: Normal is this build's only stored \
                                      style; nothing changes"
                                .into(),
                        }],
                        measurement: None,
                        point_report: None,
                    }),
                    RevCloudStyle::Calligraphy => Err(unsupported(
                        "style_unsupported",
                        "REVCLOUD Style Calligraphy: the calligraphy style is a brush-stroke \
                         rendering of the arcs, and this build's cloud is a polyline whose only \
                         per-segment shape parameter is the bulge (one circular arc); no stored \
                         field can carry the style, so the command is refused instead of being \
                         drawn as Normal (Normal remains the only cloud style)",
                    )),
                }
            }
            RevisionCloudMode::Object => self.revcloud_object(session, command, &options),
            RevisionCloudMode::Rectangular {
                first,
                second,
                reverse,
            } => {
                validate_build_points(&[*first, *second])?;
                let min_x = first.x.min(second.x);
                let max_x = first.x.max(second.x);
                let min_y = first.y.min(second.y);
                let max_y = first.y.max(second.y);
                if max_x - min_x <= 1e-12 || max_y - min_y <= 1e-12 {
                    return Err(invalid(
                        "degenerate",
                        "the two rectangle corners must span a positive width and height",
                    ));
                }
                let z = first.z;
                let points = vec![
                    Point3 {
                        x: min_x,
                        y: min_y,
                        z,
                    },
                    Point3 {
                        x: max_x,
                        y: min_y,
                        z,
                    },
                    Point3 {
                        x: max_x,
                        y: max_y,
                        z,
                    },
                    Point3 {
                        x: min_x,
                        y: max_y,
                        z,
                    },
                ];
                let (chord, source) = chord_length(
                    options.arc_length,
                    session_token,
                    bounding_diagonal(&points),
                )?;
                let geometry = resample_cloud(points, true, chord, *reverse)?;
                let mut outcome =
                    self.commit_new_entity(session, command, geometry, "revision cloud")?;
                outcome.diagnostics.push(Diagnostic {
                    object: outcome.objects.first().copied(),
                    code: "revcloud.applied".into(),
                    message: format!(
                        "REVCLOUD Rectangular: built a closed cloud with chord {chord} from the \
                         {} and reverse {reverse}",
                        source.label()
                    ),
                });
                Ok(outcome)
            }
            RevisionCloudMode::Polygonal {
                points,
                closed,
                reverse,
            }
            | RevisionCloudMode::Freehand {
                points,
                closed,
                reverse,
            } => {
                validate_build_points(points)?;
                let (chord, source) =
                    chord_length(options.arc_length, session_token, bounding_diagonal(points))?;
                let geometry = resample_cloud(points.clone(), *closed, chord, *reverse)?;
                let mut outcome =
                    self.commit_new_entity(session, command, geometry, "revision cloud")?;
                outcome.diagnostics.push(Diagnostic {
                    object: outcome.objects.first().copied(),
                    code: "revcloud.applied".into(),
                    message: format!(
                        "REVCLOUD {}: built a {} cloud with chord {chord} from the {} and reverse \
                         {reverse}",
                        if *closed { "closed path" } else { "open path" },
                        if *closed { "closed" } else { "open" },
                        source.label()
                    ),
                });
                Ok(outcome)
            }
            RevisionCloudMode::Modify {
                target,
                add_points,
                erase_side,
            } => self.revcloud_modify(
                command,
                target,
                add_points,
                *erase_side,
                &options,
                session_token,
            ),
        }
    }

    /// `REVCLOUD Object`: convert the one selected path entity.
    fn revcloud_object(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        options: &RevisionCloudOptions,
    ) -> CadResult<CommandOutcome> {
        let references = session.selection.refs().to_vec();
        let reference = match references.as_slice() {
            [] => {
                return Err(invalid(
                    "selection",
                    "REVCLOUD Object mode converts exactly one selected path, got none selected",
                ));
            }
            [one] => one.clone(),
            many => {
                return Err(invalid(
                    "selection",
                    format!(
                        "REVCLOUD Object mode converts exactly one selected path, got {} selected",
                        many.len()
                    ),
                ));
            }
        };
        if reference.document != command.document {
            return Err(invalid(
                "not_addressable",
                format!(
                    "the selected ref for entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "REVCLOUD needs an addressable model-space path; entity {} is referenced \
                     through a block instance or a sub-element",
                    reference.entity.0
                ),
            ));
        }
        let document_id = command.document;
        let entity = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .entity(reference.entity)
            .cloned()
            .ok_or_else(|| {
                invalid(
                    "target_missing",
                    format!(
                        "entity {} does not exist in this drawing",
                        reference.entity.0
                    ),
                )
            })?;
        if entity.space != SpaceId::Model {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "REVCLOUD converts model-space paths only; entity {} is in another space",
                    entity.id.0
                ),
            ));
        }
        if !matches!(
            entity.geometry,
            SemanticGeometry::Polyline { .. }
                | SemanticGeometry::Circle { .. }
                | SemanticGeometry::Ellipse { .. }
                | SemanticGeometry::Spline { .. }
        ) {
            return Err(unsupported(
                "not_convertible",
                format!(
                    "REVCLOUD Object mode converts CIRCLE, ELLIPSE, LWPOLYLINE and SPLINE; {} is \
                     not supported",
                    class_name(&entity.geometry)
                ),
            ));
        }
        let (chord, source) = chord_length(
            options.arc_length,
            session.drafting_extras.revcloud_arc_length(),
            source_diagonal(&entity),
        )?;
        let (points, closed) = flatten_source(&entity, chord)?;
        let geometry = resample_cloud(points, closed, chord, false)?;
        let edges = match &geometry {
            SemanticGeometry::Polyline { points, .. } => points.len(),
            _ => 0,
        };
        commit_updated_cloud(
            self,
            document_id,
            entity,
            geometry,
            "revcloud",
            format!(
                "REVCLOUD Object: converted the selected path into a {} cloud with {edges} \
                 arc(s) of chord {chord} from the {}",
                if closed { "closed" } else { "open" },
                source.label()
            ),
        )
    }

    /// `REVCLOUD Modify`: add points and erase the picked side.
    fn revcloud_modify(
        &mut self,
        command: &Command,
        target: &cad_domain::SelectionRef,
        add_points: &[Point3],
        erase_side: Point3,
        options: &RevisionCloudOptions,
        session_token: f64,
    ) -> CadResult<CommandOutcome> {
        if target.document != command.document {
            return Err(invalid(
                "not_addressable",
                format!(
                    "the selected ref for entity {} belongs to another document",
                    target.entity.0
                ),
            ));
        }
        if !target.instance.0.is_empty() || target.sub_element.is_some() {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "REVCLOUD Modify needs an addressable model-space cloud; entity {} is \
                     referenced through a block instance or a sub-element",
                    target.entity.0
                ),
            ));
        }
        let document_id = command.document;
        let entity = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .entity(target.entity)
            .cloned()
            .ok_or_else(|| {
                invalid(
                    "target_missing",
                    format!("entity {} does not exist in this drawing", target.entity.0),
                )
            })?;
        if entity.space != SpaceId::Model {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "REVCLOUD Modify converts model-space clouds only; entity {} is in another \
                     space",
                    entity.id.0
                ),
            ));
        }
        if !matches!(entity.geometry, SemanticGeometry::Polyline { .. }) {
            return Err(unsupported(
                "not_polyline",
                format!(
                    "REVCLOUD Modify needs a POLYLINE cloud; {} is not supported",
                    class_name(&entity.geometry)
                ),
            ));
        }
        validate_build_points(add_points)?;
        if !is_finite_point(erase_side) {
            return Err(invalid("degenerate", "the erase-side pick is not finite"));
        }
        let (chord, source) =
            chord_length(options.arc_length, session_token, source_diagonal(&entity))?;
        let (mut points, closed) = flatten_source(&entity, chord)?;
        if !closed {
            return Err(invalid(
                "modify_open",
                "REVCLOUD Modify erases a side of a closed cloud; the selected polyline is open",
            ));
        }
        insert_add_points(&mut points, add_points);
        let cloud = resample_cloud(points, true, chord, false)?;
        let (points, bulges) = match cloud {
            SemanticGeometry::Polyline { points, bulges, .. } => (points, bulges),
            _ => {
                return Err(CadError::Invariant(
                    "REVCLOUD Modify: resampling did not produce a polyline".into(),
                ));
            }
        };
        let geometry = erase_nearest_segment(&points, &bulges, erase_side)?;
        let edges = match &geometry {
            SemanticGeometry::Polyline { points, .. } => points.len(),
            _ => 0,
        };
        commit_updated_cloud(
            self,
            document_id,
            entity,
            geometry,
            "revcloud",
            format!(
                "REVCLOUD Modify: inserted {} point(s) and erased the arc nearest the pick, \
                 leaving an open {edges}-vertex cloud with chord {chord} from the {}",
                add_points.len(),
                source.label()
            ),
        )
    }
}
