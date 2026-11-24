//! `XEDGES`: extract a body's **stored** display edges as real LINE entities.
//!
//! AutoCAD's `XEDGES` creates wireframe geometry from the edges of a 3D solid,
//! surface, mesh or region. This build already stores exactly that edge set on
//! every body as [`BodyDisplay::edges`] (world-space polylines derived from the
//! exact B-rep by the tessellator, or carried verbatim by the SKP/import
//! carriers), so the command is an **extraction**, not a second geometry kernel:
//! it reads `body.display.edges` and nothing else. The source body stays (the
//! default AutoCAD behaviour); the command only adds rows.
//!
//! # Write discipline
//!
//! Every refusal happens before `begin_drawing_transaction`, so a refused
//! `XEDGES` writes nothing: no revision bump, no undo record, no id consumed.
//! On success every produced LINE is inserted in **one** transaction with one
//! undo step, so one `UNDO` removes the whole extracted wireframe.
//!
//! # What is extracted (and what is not)
//!
//! * one `LINE` per consecutive pair of points of each stored polyline, in the
//!   stored order;
//! * a segment shorter than [`cad_db::MIN_GEOMETRY_EXTENT`] is skipped, because
//!   `cad-db` refuses a zero-length line — a body whose stored edges are all
//!   degenerate therefore reports the same [`XEDGES_NO_DISPLAY_EDGES`] refusal
//!   as one that stores no edges at all;
//! * no welding/deduplication across polylines is performed: the stored edge
//!   list is the authority, so a shared boundary emitted once per adjacent face
//!   yields one LINE per stored segment. Inventing a merge pass would be a
//!   second, unsourced edge model.
//!
//! Children keep the **source body's** layer and space (an extracted edge is
//! that body's wireframe, not a copy on the current layer) and get no explicit
//! display state, exactly like `EXPLODE`'s children. Their draw order follows
//! the shared create funnel: one past the highest model-space order.
//!
//! # Selections
//!
//! A sub-element reference (a face/edge of a body) or an instance-path member is
//! refused: only a whole body row has the stored display cache this command
//! reads. A non-body row is refused by name; a selection that stores no display
//! edges is refused with [`XEDGES_NO_DISPLAY_EDGES`] instead of silently
//! producing no geometry.

use super::*;
use crate::app_utility::require_none_payload;

/// Stable refusal: the selection stores no display edges to extract.
pub(crate) const XEDGES_NO_DISPLAY_EDGES: &str = "xedges.no_display_edges";

impl Application {
    /// `XEDGES`: copy the selected bodies' stored display edges into LINE rows.
    ///
    /// Work-only (it writes entity rows) and payload-free: the selection is the
    /// whole input, exactly like `EXPLODE`.
    pub(crate) fn extract_body_edges(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "XEDGES")?;
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(refused(
                "xedges.selection_empty",
                "XEDGES needs at least one selected 3D body",
            ));
        }
        if refs
            .iter()
            .any(|reference| reference.sub_element.is_some() || !reference.instance.0.is_empty())
        {
            return Err(refused(
                "xedges.sub_element_not_supported",
                "XEDGES extracts a whole body's stored display edges; a sub-element or \
                 instance-path reference is not a body",
            ));
        }

        let document_id = command.document;
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let (sources, mut diagnostics) = Self::resolve_selection(
            &candidate,
            document_id,
            &refs,
            "xedges",
            "XEDGES",
            "xedges",
            false,
        )?;

        // Plan every segment before any write: XEDGES is all-or-nothing.
        let mut bodies = 0usize;
        let mut segments: Vec<(LayerId, SpaceId, Point3, Point3)> = Vec::new();
        for source in &sources {
            let SemanticGeometry::Body3d(body) = &source.geometry else {
                return Err(refused(
                    "xedges.not_a_body",
                    format!(
                        "entity {} is a row of type {} that is not a 3D body; XEDGES extracts the \
                         stored display edges of bodies",
                        source.id.0, source.object.type_key
                    ),
                ));
            };
            bodies += 1;
            for polyline in &body.display.edges {
                for pair in polyline.windows(2) {
                    let (start, end) = (pair[0], pair[1]);
                    // `cad-db` refuses a zero-length line; a repeated point in a
                    // display polyline is a seam, not extractable geometry.
                    if distance(start, end) < cad_db::MIN_GEOMETRY_EXTENT {
                        continue;
                    }
                    segments.push((source.layer, source.space.clone(), start, end));
                }
            }
        }
        if segments.is_empty() {
            return Err(refused(
                XEDGES_NO_DISPLAY_EDGES,
                format!(
                    "the selection stores no extractable display edges ({bodies} selected \
                     body/bodies)"
                ),
            ));
        }

        // Fresh ids and ascending draw orders, one past the highest model-space
        // order (the same rule the shared create funnel uses).
        let base_order = next_draw_order(&candidate);
        let mut entities: Vec<DbEntity> = Vec::with_capacity(segments.len());
        for (index, (layer, space, start, end)) in segments.into_iter().enumerate() {
            let geometry = SemanticGeometry::Line { start, end };
            let entity_id = candidate.allocate_entity_id();
            entities.push(DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(entity_id.0),
                    type_key: class_name(&geometry).to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: entity_id,
                layer,
                space,
                geometry,
                draw_order: base_order + index as i64,
            });
        }

        // One transaction: insert every extracted LINE. Nothing is deleted, so
        // the undo record is insert-patches only.
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged =
                candidate.begin_drawing_transaction("extract body edges", transaction)?;
            for entity in &entities {
                staged.insert_entity(entity.clone())?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = entities
            .iter()
            .map(|entity| drawing_patch(entity.id, None, Some(entity.clone())))
            .collect();

        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "extract body edges".into(),
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

        let objects: Vec<ObjectId> = entities
            .iter()
            .map(|entity| ObjectId(entity.id.0))
            .collect();
        diagnostics.push(Diagnostic {
            object: None,
            code: "drawing.xedges".into(),
            message: format!(
                "XEDGES: {} LINE(s) extracted from the stored display edges of {bodies} body/bodies",
                objects.len()
            ),
        });
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// A plan-level XEDGES refusal: `InvalidInput` carrying its stable code.
fn refused(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// One past the highest **model-space** draw order (the create funnel's rule).
fn next_draw_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .entities()
        .filter(|entity| entity.space == SpaceId::Model)
        .map(|entity| entity.draw_order)
        .max()
        .map(|order| order.saturating_add(1))
        .unwrap_or(0)
}
