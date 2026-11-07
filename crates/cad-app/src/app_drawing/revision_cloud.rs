//! `REVCLOUD` v1 — the **Object mode**: turn one selected closed,
//! straight-segment LWPOLYLINE into a closed revision-cloud polyline.
//!
//! ## Why a selection-based command
//!
//! AutoCAD's `REVCLOUD` is an interactive draw command (a rectangle or a polygon
//! is picked and the cloud follows it). This build's fixed-step capture machine
//! cannot express that prompt, so v1 implements AutoCAD's *Object* mode only: the
//! current selection is the whole input ([`CommandPayload::None`]), exactly like
//! [`CommandId::FilletPolyline`] / `JOIN`. There is **no** rectangular or
//! polygonal draw mode, no `Arc length` option and no interactive object picking.
//!
//! ## The construction
//!
//! Every edge of the closed ring is replaced by exactly one arc whose **bulge
//! magnitude** is [`REVCLOUD_BULGE`] (`0.5`, i.e. a sweep of
//! `4 * atan(0.5) ≈ 106.26°`). The vertices themselves do not move; only the
//! per-vertex bulges change, so the new geometry has the same point count as the
//! input and `closed == true`.
//!
//! The **sign** follows the ring's winding so the cloud bulges **away from the
//! polygon interior**. In this build `bulge = tan(sweep / 4)` with a positive
//! sweep counter-clockwise in the `+Z` plane (the single convention
//! [`cad_geometry::bulge_arc_geometry`] and the tessellator implement): on a
//! counter-clockwise ring (positive [`cad_geometry::signed_area`]) the arc that
//! bows to the right of its chord — the outside — is the **positive** bulge, so
//! every edge is `+0.5`; a clockwise ring gets `-0.5`. (This is pinned by the
//! `cad_geometry::area` contract: a `+1.0` bulge on the bottom edge of a
//! counter-clockwise rectangle adds area, i.e. bulges outside.)
//!
//! ## Refusals (all named, all ASCII, all before any write)
//!
//! | code | meaning |
//! | --- | --- |
//! | `revcloud.payload` | a payload was supplied (the selection is the input) |
//! | `revcloud.selection` | the selection is not exactly one entity |
//! | `revcloud.not_addressable` | another document's ref, a block-instance member, a sub-element, or a non-model-space entity |
//! | `revcloud.target_missing` | the referenced entity does not exist |
//! | `revcloud.not_polyline` | the pick is not an LWPOLYLINE (`Unsupported`) |
//! | `revcloud.open_polyline` | an open polyline (`Unsupported`) |
//! | `revcloud.too_few_vertices` | fewer than three vertices |
//! | `revcloud.bulged_segment` | a segment is already an arc (`\|bulge\| > 1e-9`, `Unsupported`) |
//! | `revcloud.degenerate` | a non-finite vertex or a zero-area ring |
//! | `revcloud.not_planar` | the vertices are not coplanar on one `+Z` plane |
//!
//! ## The write
//!
//! One `update_entity` in one transaction / one undo step. The entity keeps its
//! id, layer, space, draw order, type key and every non-geometry field, so UNDO
//! restores the straight closed polyline exactly. The success diagnostic is
//! `revcloud.applied` and names the edge count.

use super::*;
use cad_geometry::{is_xy_parallel, signed_area};

/// The bulge magnitude every edge of the revision cloud carries. AutoCAD's cloud
/// style is arc-length based; v1 fixes one arc per edge instead (declared
/// boundary), so this is the whole shape parameter.
const REVCLOUD_BULGE: f64 = 0.5;

/// A segment whose bulge exceeds this magnitude is an arc; v1 refuses to
/// overwrite it instead of silently straightening what the user drew.
const REVCLOUD_STRAIGHT_TOLERANCE: f64 = 1e-9;

/// The smallest polygon area (world units squared) that is not degenerate.
const REVCLOUD_MIN_AREA: f64 = 1e-12;

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

impl Application {
    /// `REVCLOUD` Object mode v1: rewrite the one selected closed
    /// straight-segment polyline as a revision cloud (one outward arc bulge per
    /// edge) in place.
    ///
    /// The selection is the whole input, so a payload is refused by name. Every
    /// shape requirement is checked before the drawing transaction starts; a
    /// refusal changes no revision and records no history.
    pub(crate) fn revision_cloud(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "payload",
                "REVCLOUD takes no payload; the selected polyline is the input",
            ));
        }
        let references = session.selection.refs();
        match references {
            [] => {
                return Err(invalid(
                    "selection",
                    "REVCLOUD converts exactly one closed polyline, got none selected",
                ));
            }
            [_] => {}
            many => {
                return Err(invalid(
                    "selection",
                    format!(
                        "REVCLOUD converts exactly one closed polyline, got {} selected",
                        many.len()
                    ),
                ));
            }
        }
        let reference = &references[0];
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
                    "REVCLOUD needs an addressable model-space polyline; entity {} is \
                     referenced through a block instance or a sub-element",
                    reference.entity.0
                ),
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
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
                    "REVCLOUD v1 converts model-space polylines only; entity {} is in another \
                     space",
                    reference.entity.0
                ),
            ));
        }

        let (points, bulges, closed) = match &entity.geometry {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => (points.clone(), bulges.clone(), *closed),
            other => {
                return Err(unsupported(
                    "not_polyline",
                    format!(
                        "REVCLOUD needs a POLYLINE selection; {} is not supported",
                        class_name(other)
                    ),
                ));
            }
        };
        if !closed {
            return Err(unsupported(
                "open_polyline",
                "REVCLOUD needs a closed polyline; an open polyline has no enclosed interior \
                 to place the outward arcs against",
            ));
        }
        if points.len() < 3 {
            return Err(invalid(
                "too_few_vertices",
                format!(
                    "REVCLOUD needs at least three vertices, got {}",
                    points.len()
                ),
            ));
        }
        // The stored data uses one bulge per vertex; a segment is straight when
        // its bulge is zero. An arc segment is refused, never overwritten.
        for (index, bulge) in bulges.iter().enumerate() {
            if !bulge.is_finite() || bulge.abs() > REVCLOUD_STRAIGHT_TOLERANCE {
                return Err(unsupported(
                    "bulged_segment",
                    format!(
                        "polyline segment {index} is already an arc (bulge {bulge}); only closed \
                         polylines whose every segment is straight can be converted"
                    ),
                ));
            }
        }
        if !points.iter().all(|point| is_finite_point(*point)) {
            return Err(invalid(
                "degenerate",
                "the polyline has a non-finite vertex",
            ));
        }
        // The same `+Z` coplanarity predicate the tessellator/EXPLODE use to read
        // a bulge: a non-`+Z` ring has no well-defined outward side in this build.
        if !is_xy_parallel(&points) {
            return Err(invalid(
                "not_planar",
                "the polyline vertices are not coplanar on one +Z plane; only +Z planar \
                 polylines can be converted",
            ));
        }
        let area = signed_area(&points);
        if !area.is_finite() || area.abs() <= REVCLOUD_MIN_AREA {
            return Err(invalid(
                "degenerate",
                format!(
                    "the polyline encloses no area (signed area {area}); there is no interior \
                     to bulge outward from"
                ),
            ));
        }
        // CCW (positive signed area): the interior is left of every chord, so the
        // arc bows right — outward — and in this build's `bulge = tan(sweep/4)`
        // convention that is the positive bulge. CW is the mirror.
        let bulge = if area > 0.0 {
            REVCLOUD_BULGE
        } else {
            -REVCLOUD_BULGE
        };
        let edges = points.len();
        let after_geometry = SemanticGeometry::Polyline {
            points,
            bulges: vec![bulge; edges],
            closed: true,
        };

        let entity_id = entity.id;
        let mut after = entity.clone();
        after.geometry = after_geometry;
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("revcloud", transaction)?;
            staged.update_entity(after)?;
            staged.commit()?
        };
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let stored = drawing.entity(entity_id).cloned().ok_or_else(|| {
            CadError::Invariant("REVCLOUD: the polyline vanished after commit".into())
        })?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "revcloud".to_string(),
                patches: vec![drawing_patch(entity_id, Some(entity), Some(stored))],
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(entity_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity_id.0)),
                code: "revcloud.applied".into(),
                // Locale-neutral technical diagnostic: English ASCII only.
                message: format!(
                    "REVCLOUD: replaced {edges} straight edge(s) of entity {} with one outward \
                     arc bulge of magnitude {REVCLOUD_BULGE} (sweep {} rad) each",
                    entity_id.0,
                    4.0 * REVCLOUD_BULGE.atan()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
