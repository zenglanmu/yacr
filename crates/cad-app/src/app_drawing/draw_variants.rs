//! The 2026-11-07 draw variants: WIPEOUT and `POLYGON CIRCUMSCRIBED`.
//!
//! Split out of `drawing_create.rs` (which is at the file-size ceiling) so both
//! commands keep their full contracts next to each other. The methods are inherent
//! methods on [`Application`] in this same crate, so the dispatch in
//! `drawing_create::drawing_command` and the shared `commit_new_entity` funnel are
//! unchanged.

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
    ///   (zero width is the only width the model has), and `erase` deletes it in
    ///   the same transaction;
    /// * [`WipeoutPayload::Frames`] — `Frames ON|OFF|Display but not plot`.
    ///   This is session/drawing-wide state (`WIPEOUTFRAME` 1/0/2) applied to
    ///   every wipeout. The mask always renders; the frame/plot split is recorded
    ///   state because this build emits no wipeout frame geometry and its vector
    ///   plot path has no frame channel (declared display-only).
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
                diagnostics: vec![
                    Diagnostic {
                        object: None,
                        code: "wipeout.frames".into(),
                        message: format!(
                            "WIPEOUT Frames {}: WIPEOUTFRAME = {} applies to every wipeout in \
                             the drawing",
                            frames.label(),
                            frames.value()
                        ),
                    },
                    Diagnostic {
                        object: None,
                        code: "wipeout.frames_display_only".into(),
                        message: "WIPEOUT Frames: the state is stored and reported, but this \
                                  build draws no wipeout frame geometry and its vector plot path \
                                  has no frame channel; the mask itself always renders"
                            .into(),
                    },
                ],
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
            } => {
                if !closed {
                    return Err(CadError::InvalidInput(
                        "wipeout.polyline_open: the Polyline option needs a closed polyline".into(),
                    ));
                }
                if bulges.iter().any(|bulge| bulge.abs() > 1e-9) {
                    return Err(CadError::Unsupported(
                        "wipeout.polyline_bulged: the Polyline option needs straight segments \
                         only; zero width is the only width this build models"
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
    /// IMPLEMENTED**.
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
        let vertices = crate::draw_tool::circumscribed_polygon_vertices(
            center,
            tangent_point,
            session.polygon_sides(),
        )?;
        let geometry = SemanticGeometry::Polyline {
            bulges: vec![0.0; vertices.len()],
            points: vertices,
            closed: true,
        };
        self.commit_new_entity(session, command, geometry, "create circumscribed polygon")
    }
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
