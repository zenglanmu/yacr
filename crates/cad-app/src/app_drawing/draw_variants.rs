//! The 2026-11-07 draw variants: WIPEOUT, `POLYGON CIRCUMSCRIBED`, and the
//! **authored-geometry** builders of PLINE/ELLIPSE/SPLINE.
//!
//! Split out of `drawing_create.rs` (which is at the file-size ceiling) so both
//! commands keep their full contracts next to each other. The methods are inherent
//! methods on [`Application`] in this same crate, so the dispatch in
//! `drawing_create::drawing_command` and the shared `commit_new_entity` funnel are
//! unchanged.
//!
//! The three builders at the bottom ([`authored_polyline_geometry`],
//! [`authored_ellipse_geometry`], [`authored_spline_geometry`]) are the single
//! validation point for the **programmatic** `CommandPayload::Geometry` arm of
//! PLINE, ELLIPSE and SPLINE. They author the shapes the *interactive* capture
//! cannot express yet (bulged/closed polylines, ellipse arcs and rotated ellipses,
//! CV splines of degree 2/3) while refusing — never silently rewriting — every
//! input outside the modeled 2D domain, with a stable `<selector>.<reason>` code
//! and no write. The click-capture arms of the same commands are untouched: they
//! keep their own fixed construction (see `drawing_create`).

use super::*;
use crate::command_payload::{UtilityPayload, WipeoutPayload};

impl Application {
    /// Insert a closed polygon **mask** (`WIPEOUT`) into model space, convert a
    /// closed straight-segment polyline, or set the session frame state.
    ///
    /// Payloads:
    ///
    /// * [`CommandPayload::Points`] / [`WipeoutPayload::Points`] — the existing
    ///   **closed implicit ring** of the mask: at least three finite,
    ///   `+Z`-coplanar vertices in click order, with the first vertex **not**
    ///   repeated at the end;
    /// * [`WipeoutPayload::Polyline`] — `Polyline` → `Erase Polyline [Yes/No]`.
    ///   The target must be a closed `LWPOLYLINE` whose every segment is straight
    ///   (its uniform width is irrelevant to the mask boundary), and `erase` deletes it in
    ///   the same transaction;
    /// * [`WipeoutPayload::Frames`] — `Frames ON|OFF|Display but not plot`.
    ///   This is session/drawing-wide state (`WIPEOUTFRAME` 1/0/2) applied to
    ///   every wipeout. The mask fill always renders; `ON` additionally draws and
    ///   plots the boundary frame, `OFF` hides it, and `Display but not plot`
    ///   draws it on screen while the vector plot path (built with the frame
    ///   flag clear) emits no frame. The state is display-only: it never writes
    ///   the drawing, bumps the revision or creates an undo entry.
    ///
    /// The free point-run form uses the shared [`Self::commit_new_entity`] funnel
    /// (one entity, one transaction, one undo step, active layer). The `Polyline`
    /// form inserts and optionally deletes in one custom transaction, with the
    /// deleted row's display state captured for undo. Every refusal
    /// (`wipeout.*`) happens before the transaction.
    pub(crate) fn create_wipeout(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let utility = utility_payload(command);
        if let Some(UtilityPayload::Wipeout(WipeoutPayload::Frames(frames))) = utility {
            session.drafting_extras.set_wipeout_frames(*frames);
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "wipeout.frames".into(),
                    message: format!(
                        "WIPEOUT Frames {}: WIPEOUTFRAME = {} applies to every wipeout in \
                         the drawing",
                        frames.label(),
                        frames.value()
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        if let Some(UtilityPayload::Wipeout(WipeoutPayload::Polyline { target, erase })) = utility {
            return self.create_wipeout_from_polyline(session, command, target, *erase);
        }
        let points = match (utility, &command.payload) {
            (Some(UtilityPayload::Wipeout(WipeoutPayload::Points(points))), _) => points.clone(),
            (Some(_), _) => {
                return Err(CadError::InvalidInput(
                    "wipeout.payload: WIPEOUT does not accept another utility command's payload"
                        .into(),
                ));
            }
            (None, CommandPayload::Points(points)) => points.clone(),
            (None, _) => {
                return Err(CadError::InvalidInput(
                    "wipeout.payload: WIPEOUT needs a Points([boundary…]) payload, a Polyline \
                     target or a Frames option"
                        .into(),
                ));
            }
        };
        let geometry = wipeout_geometry(points)?;
        self.commit_new_entity(session, command, geometry, "create wipeout")
    }

    /// `WIPEOUT Polyline`: convert one closed straight-segment polyline.
    fn create_wipeout_from_polyline(
        &mut self,
        session: &SessionState,
        command: &Command,
        target: &SelectionRef,
        erase: bool,
    ) -> CadResult<CommandOutcome> {
        if target.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "wipeout.polyline_not_addressable: target {} belongs to another document",
                target.entity.0
            )));
        }
        if !target.instance.0.is_empty() || target.sub_element.is_some() {
            return Err(CadError::Unsupported(format!(
                "wipeout.polyline_not_addressable: target {} is a block-instance member or \
                 sub-element; only directly addressable model-space polylines are supported",
                target.entity.0
            )));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let source = candidate.entity(target.entity).cloned().ok_or_else(|| {
            CadError::InvalidInput(format!(
                "wipeout.polyline_target_missing: target {} does not exist",
                target.entity.0
            ))
        })?;
        if source.space != SpaceId::Model {
            return Err(CadError::Unsupported(format!(
                "wipeout.polyline_not_addressable: target {} is not model space",
                source.id.0
            )));
        }
        let (boundary, closed) = match &source.geometry {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
                ..
            } => {
                if !closed {
                    return Err(CadError::InvalidInput(
                        "wipeout.polyline_open: the Polyline option needs a closed polyline".into(),
                    ));
                }
                if bulges.iter().any(|bulge| bulge.abs() > 1e-9) {
                    return Err(CadError::Unsupported(
                        "wipeout.polyline_bulged: the Polyline option needs straight segments \
                         only; per-vertex taper is not modeled"
                            .into(),
                    ));
                }
                (points.clone(), *closed)
            }
            other => {
                return Err(CadError::Unsupported(format!(
                    "wipeout.polyline_not_polyline: {} is not an LWPOLYLINE",
                    class_name(other)
                )));
            }
        };
        if !closed {
            return Err(CadError::InvalidInput(
                "wipeout.polyline_open: the Polyline option needs a closed polyline".into(),
            ));
        }
        let geometry = wipeout_geometry(boundary)?;
        let target_display = erase.then(|| candidate.entity_display_state(source.id));
        let new_id = candidate.allocate_entity_id();
        let base_order = candidate
            .entities()
            .filter(|entity| entity.space == SpaceId::Model)
            .map(|entity| entity.draw_order)
            .max()
            .map(|order| order.saturating_add(1))
            .unwrap_or(0);
        let mask = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(new_id.0),
                type_key: class_name(&geometry).to_string(),
                revision: Revision(0),
                source_handle: None,
            },
            id: new_id,
            layer: session.active_layer,
            space: SpaceId::Model,
            geometry,
            draw_order: base_order,
        };
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged =
                candidate.begin_drawing_transaction("wipeout polyline", transaction)?;
            staged.insert_entity(mask.clone())?;
            if erase {
                staged.delete_entity(source.id)?;
            }
            staged.commit()?
        };
        let stored = candidate
            .entity(new_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("WIPEOUT: the mask vanished after commit".into()))?;
        let mut patches = vec![drawing_patch(new_id, None, Some(stored))];
        if erase {
            patches.push(drawing_patch_deleted(
                source.id,
                source.clone(),
                target_display.expect("captured when erase is true"),
            ));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "wipeout polyline".into(),
                patches,
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
            objects: vec![ObjectId(new_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(new_id.0)),
                code: "wipeout.polyline".into(),
                message: format!(
                    "WIPEOUT Polyline: converted the closed straight polyline into one \
                     AcDbWipeout mask; Erase Polyline = {}",
                    if erase { "Yes" } else { "No" }
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// Insert a closed regular POLYGON **circumscribed about a circle**, as one
    /// entity.
    ///
    /// Payload is `Points([center, tangent_point])`: the centre and a point on the
    /// **inscribed (tangent) circle** — the circle the polygon's edges touch —
    /// exactly as clicked (the construction input, like [`Self::create_polygon`]).
    /// The side count is the session sysvar
    /// ([`SessionState::polygon_sides`], `3..=1024`), read here rather than carried
    /// on the payload, so the session stays the single authority.
    ///
    /// The geometry is built by
    /// [`crate::draw_tool::circumscribed_polygon_vertices`] — the single
    /// construction/validation point shared with the live preview. It derives the
    /// tangent radius `r` from the second pick, the vertex radius
    /// `R = r / cos(π/n)`, and places the vertices at `θ ± π/n` (the picked
    /// direction is the **midpoint of the first edge**), stepping counter-clockwise
    /// about `+Z`. Every edge's midpoint is exactly `r` from the centre, the ring is
    /// closed with one zero bulge per vertex, and the refusals are
    /// `polygoncircumscribed.invalid_point` / `.sides` / `.not_planar` /
    /// `.zero_radius`. Committed through the shared [`Self::commit_new_entity`]
    /// funnel: one entity, one transaction and one undo step, active-layer check
    /// intact.
    ///
    /// Only this circumscribed construction and the existing inscribed
    /// [`Self::create_polygon`] are authored; AutoCAD's `Edge` branch and a numeric
    /// radius that rotates the ring by the snap rotation angle remain **NOT
    /// IMPLEMENTED** — no payload field or session channel can request either, so
    /// both keep their documented refusal-free absence (see `create_polygon`).
    pub(crate) fn create_polygon_circumscribed(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreatePolygonCircumscribed needs a Points([center, tangent_point]) payload".into(),
            ));
        };
        let center = *points.first().ok_or_else(|| {
            CadError::InvalidInput("CreatePolygonCircumscribed needs a center point".into())
        })?;
        let tangent_point = *points.get(1).ok_or_else(|| {
            CadError::InvalidInput("CreatePolygonCircumscribed needs a tangent-circle point".into())
        })?;
        // The construction is exactly two picks. A longer list would be silently
        // dropped before this guard existed, which is a fabricated success for a
        // caller who passed more input than the command reads.
        if points.len() > 2 {
            return Err(draw_refusal(
                "polygoncircumscribed",
                "payload_length",
                format!(
                    "the circumscribed construction takes exactly two picks (centre and \
                     tangent-circle point), got {}",
                    points.len()
                ),
            ));
        }
        let vertices = crate::draw_tool::circumscribed_polygon_vertices(
            center,
            tangent_point,
            session.polygon_sides(),
        )?;
        let geometry = SemanticGeometry::Polyline {
            bulges: vec![0.0; vertices.len()],
            points: vertices,
            closed: true,
            width: 0.0,
        };
        self.commit_new_entity(session, command, geometry, "create circumscribed polygon")
    }

    /// Insert an ELLIPSE by the axis-endpoint method as one entity.
    ///
    /// Payload choice: `Points([p1, p2, p3])` — the two endpoints of the first
    /// axis and the third pick that gives the other half-axis distance, exactly
    /// as clicked (the construction input, like `CreateCircle`/`CreateRectangle`).
    /// The geometry is built by [`crate::draw_tool::ellipse_parameters`] — the
    /// single construction/validation point shared with the preview — which
    /// stores the **longer** semi-axis as `major_axis` (`ratio <= 1`) and refuses
    /// a non-finite, degenerate (coincident first-axis endpoints, or a third pick
    /// on the first axis) or non-coplanar triple. The click path authors the full
    /// ellipse (`start = 0`, `sweep = 2π`) on the work-plane `+Z` normal. More than
    /// three picks is refused (`ellipse.payload_length`): the construction is
    /// exactly three, and a longer list used to be silently truncated into a full
    /// ellipse.
    ///
    /// The programmatic payload `Geometry(SemanticGeometry::Ellipse { center,
    /// normal, major_axis, ratio, start, sweep })` authors the **full modeled
    /// ellipse family** through [`authored_ellipse_geometry`], the single
    /// validation point:
    ///
    /// * **arc mode** — any `start`/`sweep` with `0 < |sweep| < 2π` is an ellipse
    ///   arc (a negative sweep is the clockwise direction), the same parameters the
    ///   DXF ELLIPSE entity and the renderer already read;
    /// * **rotation about the major axis** — representable **faithfully**: DXF
    ///   expresses a rotation `R` about the major axis as the tilted `normal`
    ///   (rotated by `R` about `major_axis`) plus the projected ratio
    ///   `ratio · cos R`, and the orthographic projection of that tilted ellipse is
    ///   exactly the `+Z` ellipse with the same major axis and `ratio · cos R`
    ///   (verified in the domain and pinned by
    ///   `tests/drawing/ellipse.rs::ellipse_rotation_about_the_major_axis_is_the_projection_rule`).
    ///   So the rotated ellipse is stored as its own frame, never squashed into a
    ///   fabricated flat ellipse; `R = 90°` collapses the minor axis and is refused
    ///   as `ellipse.ratio` / `ellipse.sweep` semantics rather than stored.
    ///
    /// Every authored refusal (`ellipse.<reason>`) is listed on
    /// [`authored_ellipse_geometry`] and happens before a transaction is opened.
    /// The ellipse is committed through the shared [`Self::commit_new_entity`]
    /// funnel, so it is one entity, one transaction and one undo step, with the
    /// active-layer check intact.
    pub(crate) fn create_ellipse(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(points) => {
                let p1 = *points
                    .first()
                    .ok_or_else(|| CadError::InvalidInput("CreateEllipse needs p1".into()))?;
                let p2 = *points
                    .get(1)
                    .ok_or_else(|| CadError::InvalidInput("CreateEllipse needs p2".into()))?;
                let p3 = *points
                    .get(2)
                    .ok_or_else(|| CadError::InvalidInput("CreateEllipse needs p3".into()))?;
                if points.len() > 3 {
                    return Err(CadError::InvalidInput(format!(
                        "ellipse.payload_length: the axis-endpoint construction takes exactly \
                         three picks (two axis endpoints and one other-axis point); an ellipse \
                         arc needs the stored start/sweep through a Geometry(Ellipse) payload, \
                         got {} picks",
                        points.len()
                    )));
                }
                let frame = crate::draw_tool::ellipse_parameters(p1, p2, p3)?;
                SemanticGeometry::Ellipse {
                    center: frame.center,
                    // The axis-endpoint method is a work-plane construction; a full
                    // ellipse uses +Z, never a fabricated tilt.
                    normal: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 1.0,
                    },
                    major_axis: frame.major_axis,
                    ratio: frame.ratio,
                    start: 0.0,
                    sweep: std::f64::consts::TAU,
                }
            }
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Ellipse {
                    center,
                    normal,
                    major_axis,
                    ratio,
                    start,
                    sweep,
                } => authored_ellipse_geometry(
                    *center,
                    *normal,
                    *major_axis,
                    *ratio,
                    *start,
                    *sweep,
                )?,
                other => {
                    return Err(CadError::InvalidInput(format!(
                        "CreateEllipse needs an Ellipse geometry, got {}",
                        geometry_kind(other)
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "CreateEllipse needs a Points([p1, p2, p3]) or Geometry(Ellipse) payload"
                        .into(),
                ));
            }
        };
        self.commit_new_entity(session, command, geometry, "create ellipse")
    }

    /// Draw a SPLINE: the fit-point (interpolating) method or the **CV method**.
    ///
    /// The click-capture payload `Points(fit_points)` — the ordered fit points the
    /// user clicked (the construction input, like `CreateCircle`/`CreatePolygon`) —
    /// is delegated to [`crate::draw_tool::spline_control_points`], the pure geometry
    /// core, so the handler never reimplements it: fewer than four fit points, a
    /// non-finite or non-coplanar `+Z` point, a mismatched payload, and every
    /// degenerate-chord / numerically singular input the core raises are explicit
    /// refusals that create nothing. The stored spline is degree 3 with clamped chord
    /// knots and unit weights (fit points do **not** round-trip: the domain has no
    /// fit-point field).
    ///
    /// The programmatic payload `Geometry(SemanticGeometry::Spline { degree, knots,
    /// control_points, weights })` authors the **CV (control vertex) method** through
    /// [`authored_spline_geometry`], the single validation point: the picked control
    /// points are stored **verbatim** (not interpolated into fit points), with
    /// **degree 2 or 3**, and the knot vector/weights as given (empty vectors are the
    /// documented clamped-uniform / all-ones shorthands; the structural relations are
    /// enforced by the database's one write validator).
    ///
    /// Deliberate gaps, refused by absence rather than by a fabricated entity:
    /// **`Close`** (a periodic closed spline needs a closed flag the domain does not
    /// model), **`Tangency`** (start/end tangent picks), the **`Knots`**
    /// parameterization prompt (the fit path is chord, documented) and **`Object`**
    /// (convert an object into a spline) have no payload field and no session
    /// parameter in this build, so none can be requested.
    ///
    /// Both arms are committed through the shared [`Self::commit_new_entity`]
    /// funnel: one entity, one transaction, one undo step, active-layer check
    /// intact.
    pub(crate) fn create_spline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(fit_points) => {
                let (knots, control_points) = crate::draw_tool::spline_control_points(fit_points)?;
                let weights = vec![1.0; control_points.len()];
                SemanticGeometry::Spline {
                    degree: cad_geometry::FIT_SPLINE_DEGREE,
                    knots,
                    control_points,
                    weights,
                }
            }
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Spline {
                    degree,
                    knots,
                    control_points,
                    weights,
                } => authored_spline_geometry(
                    *degree,
                    knots.clone(),
                    control_points.clone(),
                    weights.clone(),
                )?,
                other => {
                    return Err(CadError::InvalidInput(format!(
                        "CreateSpline needs a Spline geometry, got {}",
                        geometry_kind(other)
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "CreateSpline needs a Points(fit_points) or Geometry(Spline) payload".into(),
                ));
            }
        };
        self.commit_new_entity(session, command, geometry, "create spline")
    }
}

/// A refused **authored** (programmatic) draw payload: the calling selector plus a
/// stable reason code.
///
/// The create commands' click-capture arms keep their older prose refusals (their
/// messages are pinned by tests); every new authored-geometry refusal uses this
/// shape so a caller can branch on `<selector>.<reason>` without parsing a
/// sentence. Nothing is written, no revision is bumped and no history entry is
/// recorded — the builders run before any transaction is opened.
fn draw_refusal(selector: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{selector}.{code}: {detail}"))
}

/// The smallest world separation between two authored polyline vertices that still
/// counts as a real chord.
///
/// Mirrors `drawing_create::MIN_POLYLINE_VERTEX_SEPARATION` (module-private there)
/// so the click-capture and authored paths agree about what "the same point" means;
/// a deliberately tiny relative-free epsilon, so only an exactly-degenerate pair is
/// refused.
const MIN_AUTHORED_VERTEX_SEPARATION: f64 = 1e-12;

/// The smallest `|bulge|` that is a real arc segment rather than a stored zero.
///
/// The same threshold `polyline_curve::PolylineCurve` and the offset/break paths use
/// to decide "straight", so a polyline authored here cannot disagree with the
/// readers that consume it.
const MIN_AUTHORED_BULGE: f64 = 1e-9;

/// Build the polyline a **programmatic** `PLINE` payload authors: arc segments
/// (stored as bulges) and a closed ring, on top of the click path's straight
/// segments.
///
/// The domain has modeled both since the import path was written —
/// `SemanticGeometry::Polyline { points, bulges, closed }` is one entity with one
/// bulge per vertex and an implicit closing segment — so this is the command layer
/// catching up with the model, not a new field. `bulges[i]` is the **tan(θ/4)** of
/// the segment from `points[i]` to the next vertex (`> 0` counter-clockwise about
/// `+Z`, `< 0` clockwise, `0` straight); a closed ring takes the segment from the
/// last vertex back to the first as its last bulge.
///
/// Documented normalisations (lossless, never a shape change):
///
/// * a `closed` ring whose last vertex repeats the first drops that redundant
///   vertex (and its bulge) — the stored convention is the **implicit ring**, the
///   one `polyline_curve::PolylineCurve` and the WIPEOUT/BOUNDARY creators already
///   assume, and the repeated vertex carries no geometry;
/// * an **empty** bulge list means "every segment straight" and is filled with one
///   `0.0` per vertex, and an open polyline may give one bulge per **segment**
///   (`n - 1`), in which case the trailing vertex — which has no outgoing segment —
///   receives the documented `0.0`.
///
/// Explicit refusals that create nothing (stable `<selector>.<reason>` codes, never
/// a silent fix-up or projection):
///
/// * `pline.too_few_points` — fewer than two vertices;
/// * `pline.invalid_point` — a non-finite vertex;
/// * `pline.non_finite_bulge` — a non-finite bulge;
/// * `pline.closed_too_few_vertices` — a closed ring with fewer than three
///   vertices (two vertices would be a doubled segment enclosing nothing);
/// * `pline.bulge_count` — a bulge list that is neither `n`, `n - 1` (open) nor
///   empty;
/// * `pline.degenerate` — fewer than two **distinct** vertices;
/// * `pline.bulged_non_planar` (`Unsupported`) — a bulged polyline off one `+Z`
///   plane: the arc segments are resolved on the work plane, so a tilted ring
///   cannot be stored faithfully;
/// * `pline.degenerate_bulge` — a non-zero bulge on a zero-length chord (the arc
///   has no endpoints to run between);
/// * `pline.width_invalid` — a negative or non-finite uniform width.
///
/// The uniform `width` is stored on the created polyline (`0.0` = hairline,
/// AutoCAD's constant LWPOLYLINE width). Per-vertex start/end taper is not
/// modeled and is never invented from a single number. Deliberate remaining gap:
/// **`Undo` of the last vertex** is a capture-state-machine step, not a command
/// payload, so it cannot be requested here (see the PLINE handler).
pub(super) fn authored_polyline_geometry(
    mut points: Vec<Point3>,
    mut bulges: Vec<f64>,
    closed: bool,
    width: f64,
) -> CadResult<SemanticGeometry> {
    if !width.is_finite() || width < 0.0 {
        return Err(draw_refusal(
            "pline",
            "width_invalid",
            format!("an authored polyline width must be finite and >= 0, got {width}"),
        ));
    }
    if points.len() < 2 {
        return Err(draw_refusal(
            "pline",
            "too_few_points",
            format!(
                "an authored polyline needs at least two vertices, got {}",
                points.len()
            ),
        ));
    }
    if !points.iter().all(|point| is_finite_point(*point)) {
        return Err(draw_refusal(
            "pline",
            "invalid_point",
            "every authored polyline vertex must be finite",
        ));
    }
    if !bulges.iter().all(|bulge| bulge.is_finite()) {
        return Err(draw_refusal(
            "pline",
            "non_finite_bulge",
            "every authored polyline bulge must be finite",
        ));
    }
    if closed {
        let last = *points.last().expect("checked non-empty");
        if distance(points[0], last) <= MIN_AUTHORED_VERTEX_SEPARATION {
            points.pop();
            if bulges.len() > points.len() {
                bulges.pop();
            }
        }
        if points.len() < 3 {
            return Err(draw_refusal(
                "pline",
                "closed_too_few_vertices",
                format!(
                    "a closed polyline needs at least three vertices, got {}",
                    points.len()
                ),
            ));
        }
    }
    let segments = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    if bulges.is_empty() {
        bulges = vec![0.0; points.len()];
    } else if bulges.len() == segments && bulges.len() < points.len() {
        // One bulge per **segment** on an open polyline: the trailing vertex has no
        // outgoing segment, so its documented bulge is the straight `0.0`.
        bulges.push(0.0);
    }
    if bulges.len() != points.len() {
        return Err(draw_refusal(
            "pline",
            "bulge_count",
            format!(
                "an authored polyline with {} vertices (closed = {closed}) takes one bulge per \
                 vertex or per segment ({segments}), got {}; a closed polyline's implicit closing \
                 segment owns the last bulge",
                points.len(),
                bulges.len()
            ),
        ));
    }
    if !points
        .iter()
        .any(|point| distance(points[0], *point) > MIN_AUTHORED_VERTEX_SEPARATION)
    {
        return Err(draw_refusal(
            "pline",
            "degenerate",
            "an authored polyline needs at least two distinct vertices",
        ));
    }
    if bulges.iter().any(|bulge| bulge.abs() > MIN_AUTHORED_BULGE) {
        // A bulge is resolved as a circular arc on the work plane (the same
        // `bulge_arc_geometry(.., +Z)` every reader uses), so an off-plane ring has
        // no faithful arc segment and is refused rather than flattened.
        if !cad_geometry::is_xy_parallel(&points) {
            return Err(CadError::Unsupported(
                "pline.bulged_non_planar: a bulged polyline must lie in one +Z plane; the arc \
                 segments are resolved on the work plane"
                    .into(),
            ));
        }
        for segment in 0..segments {
            let chord = distance(points[segment], points[(segment + 1) % points.len()]);
            if bulges[segment].abs() > MIN_AUTHORED_BULGE && chord <= MIN_AUTHORED_VERTEX_SEPARATION
            {
                return Err(draw_refusal(
                    "pline",
                    "degenerate_bulge",
                    format!("segment {segment} carries a bulge but its chord has zero length"),
                ));
            }
        }
    }
    Ok(SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        width,
    })
}

/// Build the ellipse a **programmatic** `ELLIPSE` payload authors: an **ellipse
/// arc** (`start`/`sweep` shorter than one turn) and an ellipse whose plane is
/// **rotated about its major axis**.
///
/// Both are already in the model, which is why this command can author them without
/// a new field: `SemanticGeometry::Ellipse { center, normal, major_axis, ratio,
/// start, sweep }` is the DXF ELLIPSE definition, and DXF expresses a rotation `R`
/// about the major axis exactly as a tilted `normal` (rotated by `R` about
/// `major_axis`) plus the projected `ratio' = ratio · cos R`. **Verified against
/// the domain representation** (pinned by
/// `tests/drawing/ellipse.rs::ellipse_rotation_about_the_major_axis_is_the_projection_rule`):
/// for the frame above, the orthographic projection of the tilted ellipse onto the
/// `+Z` plane is the ellipse with `+Z` normal, the same `major_axis` and
/// `ratio · cos R` — so the rotated 3D ellipse and its documented 2D projection are
/// one representable family, not an approximation. `R = 90°` collapses the minor
/// axis (`ratio · cos R = 0`) and is refused, exactly as AutoCAD's rotation prompt
/// degenerates to a segment there.
///
/// `start`/`sweep` are the stored parameter angles in radians: `0`/`2π` is the full
/// ellipse (the click path's only construction), any `|sweep| < 2π` is an ellipse
/// arc, and a **negative** sweep is the clockwise arc (AutoCAD's direction choice).
///
/// Explicit refusals that create nothing (`ellipse.<reason>`):
///
/// * `ellipse.invalid_frame` — a non-finite centre/normal/major axis, or a
///   zero-length `normal` (the plane would be undefined);
/// * `ellipse.axis` — a collapsed `major_axis`;
/// * `ellipse.ratio` — a non-finite or non-positive `ratio`, or one **greater than
///   one**: the create path keeps the DXF-legal invariant that the stored major axis
///   is the longer semi-axis (what [`crate::draw_tool::ellipse_parameters`] always
///   produces); swapping the axes is the caller's step, never a silent rewrite here;
/// * `ellipse.frame` — a `normal` parallel to `major_axis` (ambiguous plane);
/// * `ellipse.sweep` — a non-finite, zero, or larger-than-one-turn sweep (an ellipse
///   arc covers at most one full turn; `|sweep| == 2π` is the full ellipse).
pub(super) fn authored_ellipse_geometry(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
) -> CadResult<SemanticGeometry> {
    if !is_finite_point(center) || !is_finite_point(normal) || !is_finite_point(major_axis) {
        return Err(draw_refusal(
            "ellipse",
            "invalid_frame",
            "the centre, normal and major axis must be finite",
        ));
    }
    let normal_length = cad_geometry::length(normal);
    if !normal_length.is_finite() || normal_length <= MIN_AUTHORED_VERTEX_SEPARATION {
        return Err(draw_refusal(
            "ellipse",
            "invalid_frame",
            "the ellipse normal must have a non-zero, finite length",
        ));
    }
    let major_length = cad_geometry::length(major_axis);
    if !major_length.is_finite() || major_length <= MIN_AUTHORED_VERTEX_SEPARATION {
        return Err(draw_refusal(
            "ellipse",
            "axis",
            "the ellipse major axis must have a non-zero, finite length",
        ));
    }
    if !ratio.is_finite() || ratio <= 0.0 || ratio > 1.0 {
        return Err(draw_refusal(
            "ellipse",
            "ratio",
            format!(
                "the ellipse ratio must be finite and in (0, 1] (the stored major axis is the \
                 longer semi-axis), got {ratio}"
            ),
        ));
    }
    // The plane is undefined when the extrusion is parallel to the major axis.
    let cross = cad_geometry::cross(normal, major_axis);
    if !cross.x.is_finite() || !cross.y.is_finite() || !cross.z.is_finite() {
        return Err(draw_refusal(
            "ellipse",
            "invalid_frame",
            "the ellipse frame cross product must be finite",
        ));
    }
    if cad_geometry::length(cross) <= 1e-9 * normal_length * major_length {
        return Err(draw_refusal(
            "ellipse",
            "frame",
            "the ellipse normal must not be parallel to its major axis",
        ));
    }
    if !start.is_finite() || !sweep.is_finite() {
        return Err(draw_refusal(
            "ellipse",
            "sweep",
            "the ellipse start angle and sweep must be finite",
        ));
    }
    let tau = std::f64::consts::TAU;
    if sweep.abs() <= MIN_AUTHORED_BULGE {
        return Err(draw_refusal(
            "ellipse",
            "sweep",
            "the ellipse sweep must be non-zero; a zero sweep is a point, not an arc",
        ));
    }
    if sweep.abs() > tau + 1e-9 {
        return Err(draw_refusal(
            "ellipse",
            "sweep",
            format!("an ellipse arc covers at most one full turn (|sweep| <= 2π), got |{sweep}|"),
        ));
    }
    Ok(SemanticGeometry::Ellipse {
        center,
        normal,
        major_axis,
        ratio,
        start,
        sweep,
    })
}

/// Build the spline a **programmatic** `SPLINE` payload authors: the **CV (control
/// vertex) method** with an explicit **degree 2 or 3**.
///
/// The click path interpolates fit points into a degree-3 clamped spline
/// ([`crate::draw_tool::spline_control_points`]); the CV method stores the control
/// points the user picked **verbatim**, which is exactly what
/// `SemanticGeometry::Spline { degree, knots, control_points, weights }` is. The
/// knot vector and the weights are therefore taken as given: an empty `knots` (the
/// sanctioned clamped-uniform shorthand) and an empty `weights` (all ones) are
/// accepted, and the structural relations (`knots.len() == n + degree + 1`,
/// `weights.len() == n`, non-decreasing knots, positive finite weights) are enforced
/// by the database's one write validator
/// ([`cad_db::validate_geometry`]) — the same rule for every writer, surfaced
/// unchanged as this command's error. Nothing is written on any of them.
///
/// Degree policy: **2 or 3 only**
/// (`spline.degree`). Both are what AutoCAD 2026's `Degree` prompt offers for a CV
/// spline; degree 1 is a polyline's job (`PLINE`), and a higher degree has no
/// prompt in the modeled domain. At least `degree + 1` control points are required
/// (`spline.control_points`) and every one must be finite (`spline.invalid_point`) —
/// stated here rather than left to the database so the refusal names the selector.
///
/// Deliberate gaps, refused by absence: **`Close` is not modeled** —
/// `SemanticGeometry::Spline` has no closed flag (a periodic spline needs wrapped
/// knots *and* the DXF `70` bit 1, and inventing the second is exactly the
/// fabricated state this build forbids), so a closed CV spline cannot be requested
/// and no stable code exists to refuse one; likewise **`Tangency`**, the **`Knots`**
/// parameterization prompt (the fit path is chord, documented) and **`Object`**
/// (convert an object to a spline) have no payload field or session channel.
pub(super) fn authored_spline_geometry(
    degree: u32,
    knots: Vec<f64>,
    control_points: Vec<Point3>,
    weights: Vec<f64>,
) -> CadResult<SemanticGeometry> {
    if degree != 2 && degree != 3 {
        return Err(draw_refusal(
            "spline",
            "degree",
            format!("a CV spline is degree 2 or 3, got {degree}"),
        ));
    }
    if control_points.len() < degree as usize + 1 {
        return Err(draw_refusal(
            "spline",
            "control_points",
            format!(
                "a degree-{degree} spline needs at least {} control points, got {}",
                degree as usize + 1,
                control_points.len()
            ),
        ));
    }
    if !control_points.iter().all(|point| is_finite_point(*point)) {
        return Err(draw_refusal(
            "spline",
            "invalid_point",
            "every spline control point must be finite",
        ));
    }
    Ok(SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
    })
}

/// The smallest XY distance (world units) between two consecutive WIPEOUT
/// boundary vertices.
///
/// A zero-length edge has no direction and makes the implicit ring degenerate;
/// like the other world epsilons this is a tiny relative-free constant, so an
/// exactly-duplicate click pair is refused while any real edge is accepted.
const MIN_WIPEOUT_VERTEX_SEPARATION: f64 = 1e-12;

/// The smallest enclosed area (world units²) for a WIPEOUT boundary to be a real
/// region rather than a collapsed (collinear) run of points.
///
/// The comparison is on the **shoelace sum** (twice the signed area), computed in
/// a frame translated to the first vertex so it is translation-invariant at large
/// world coordinates.
const MIN_WIPEOUT_AREA: f64 = 1e-12;

/// Build the closed polygon **mask** of a WIPEOUT from its boundary vertices.
///
/// The ring is implicit: the returned `boundary` keeps exactly the vertices given
/// (the first vertex is stored once, never repeated at the end). The stored mask
/// is always `inverted: false` — masking the inside — because the create path has
/// no inverted/frame model (see
/// [`crate::command_id::draw_variants::WipeoutCommand`]).
///
/// Explicit refusals that create nothing (stable machine prefixes, never a
/// silent fix-up or projection): `wipeout.too_few_points`, `wipeout.invalid_point`,
/// `wipeout.not_planar` (the flat `+Z` work-plane subset), `wipeout.duplicate_vertex`
/// (a zero-length edge, the closing pair included) and `wipeout.zero_area` (a
/// collapsed ring). Self-intersection is deliberately **not** checked: a
/// self-crossing ring is stored as given and filled with the renderer's even-odd
/// rule, except that a ring whose net shoelace area is zero (for example a
/// symmetric bow-tie) is refused as `wipeout.zero_area` — the area test is the
/// only degeneracy test this v1 runs.
fn wipeout_geometry(points: Vec<Point3>) -> CadResult<SemanticGeometry> {
    let count = points.len();
    if count < 3 {
        return Err(CadError::InvalidInput(format!(
            "wipeout.too_few_points: WIPEOUT needs at least three boundary vertices, got {count}"
        )));
    }
    if !points.iter().all(|point| is_finite_point(*point)) {
        return Err(CadError::InvalidInput(
            "wipeout.invalid_point: every WIPEOUT boundary vertex must be finite".into(),
        ));
    }
    let first = points[0];
    let z_tolerance = 1e-9 * (1.0 + points.iter().map(|point| point.z.abs()).sum::<f64>());
    if points
        .iter()
        .any(|point| (point.z - first.z).abs() > z_tolerance)
    {
        return Err(CadError::InvalidInput(
            "wipeout.not_planar: WIPEOUT boundary vertices must lie in one +Z plane".into(),
        ));
    }
    for index in 0..count {
        let next = (index + 1) % count;
        let (a, b) = (points[index], points[next]);
        let separation = (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z);
        if separation <= MIN_WIPEOUT_VERTEX_SEPARATION {
            return Err(CadError::InvalidInput(format!(
                "wipeout.duplicate_vertex: boundary vertices {index} and {next} coincide"
            )));
        }
    }
    // Twice the signed area, relative to the first vertex (translation-invariant).
    let mut twice_area = 0.0;
    for index in 1..(count - 1) {
        let a = (points[index].x - first.x, points[index].y - first.y);
        let b = (points[index + 1].x - first.x, points[index + 1].y - first.y);
        twice_area += a.0 * b.1 - a.1 * b.0;
    }
    if !twice_area.is_finite() || twice_area.abs() <= MIN_WIPEOUT_AREA {
        return Err(CadError::InvalidInput(
            "wipeout.zero_area: the WIPEOUT boundary encloses no area".into(),
        ));
    }
    Ok(SemanticGeometry::Mask {
        boundary: points,
        inverted: false,
    })
}
