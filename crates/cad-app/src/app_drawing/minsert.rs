//! `MINSERT`: insert an existing block as a rectangular array of references.
//!
//! The typed form is `MINSERT <block> <rows> <columns> <row_spacing>
//! <column_spacing> <x> <y>`; the payload is
//! [`crate::CommandPayload::MInsert`].
//!
//! Every cell is one real `SemanticGeometry::Insert` reference to the **same**
//! existing block definition. There is no MINSERT entity in this build's domain
//! model (and no new variant is introduced here), so the array is stored as the
//! independent references AutoCAD's MINSERT expands to — one transaction, one
//! undo step, all-or-nothing.
//!
//! ## Placement rule
//!
//! The grid rule is exactly ARRAYRECT's declared rule
//! ([`Application::array_rect`]): cell `(row, column)` translates the reference by
//! `(column · column_spacing, row · row_spacing)`, so a **negative** spacing arrays
//! down/left, zero spacing is legal (the cells coincide, reported by the count) and
//! there is no invented rotation or level direction. Unlike ARRAYRECT the source
//! is **not** retained, so cell `(0, 0)` is itself an INSERT: `rows × columns`
//! references are always written.
//!
//! Each reference carries the importer's own `T(insertion) · T(−base_point)`
//! composition (the INSERT convention), so the block's base point lands exactly on
//! its cell insertion point.
//!
//! Refusals (nothing written, no transaction): `minsert.payload`,
//! `minsert.invalid_point`, `minsert.invalid_name`, `minsert.unknown_block`,
//! `minsert.block_metadata_missing`, `minsert.invalid_count`,
//! `minsert.invalid_spacing`, `minsert.too_many` (the shared
//! [`MAX_ARRAY_ENTITIES`] cap, never truncated). A locked active layer is refused
//! by the stager's own `layer.locked` chokepoint before anything is committed.

use super::*;
use cad_db::DbObject;
use cad_history::{DrawingPatch, DrawingUndoRecord};

/// An `minsert.<code>` refusal.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The next free draw order above every model-space row (the ARRAY rule).
fn next_model_draw_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .map(|max| max.saturating_add(1))
        .unwrap_or(0)
}

impl Application {
    /// Create `rows × columns` block references in one transaction.
    pub(crate) fn minsert(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::MInsert(payload) = &command.payload else {
            return Err(refusal(
                "minsert.payload",
                "MINSERT needs an MInsert payload (block, rows, columns, row_spacing, \
                 column_spacing, x, y); the typed form is `MINSERT <block> <rows> <columns> \
                 <row_spacing> <column_spacing> <x> <y>`",
            ));
        };
        let request = payload.as_ref();
        if !is_finite_point(request.insertion) {
            return Err(refusal(
                "minsert.invalid_point",
                format!(
                    "the MINSERT insertion point {:?} is not finite",
                    request.insertion
                ),
            ));
        }
        let name = request.block.trim();
        if name.is_empty() {
            return Err(refusal(
                "minsert.invalid_name",
                "MINSERT needs a non-empty block name",
            ));
        }
        if name.contains('\r') || name.contains('\n') {
            return Err(refusal(
                "minsert.invalid_name",
                "MINSERT block name must be a single line",
            ));
        }
        let characters = name.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(refusal(
                "minsert.invalid_name",
                format!(
                    "MINSERT allows at most {MAX_TOOL_TEXT_CHARS} characters in a block name, got \
                     {characters}"
                ),
            ));
        }
        if request.rows == 0 || request.columns == 0 {
            return Err(refusal(
                "minsert.invalid_count",
                format!(
                    "MINSERT rows and columns must both be at least 1, got {} x {}",
                    request.rows, request.columns
                ),
            ));
        }
        if !request.row_spacing.is_finite() || !request.column_spacing.is_finite() {
            return Err(refusal(
                "minsert.invalid_spacing",
                format!(
                    "MINSERT row/column spacing must be finite, got {} / {}",
                    request.row_spacing, request.column_spacing
                ),
            ));
        }
        let produced = request.rows as f64 * request.columns as f64;
        if !produced.is_finite() || produced > MAX_ARRAY_ENTITIES as f64 {
            return Err(refusal(
                "minsert.too_many",
                format!(
                    "MINSERT would insert {produced} references; the array cap is \
                     {MAX_ARRAY_ENTITIES}"
                ),
            ));
        }

        // Resolve the definition and its stored base point from the authoritative
        // drawing before the candidate clone mutates anything.
        let (block, block_name, base_point) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            let definition = drawing.block_by_name(name).ok_or_else(|| {
                refusal(
                    "minsert.unknown_block",
                    format!("no block named '{name}' in this drawing"),
                )
            })?;
            let metadata = drawing.block_metadata(definition.id).ok_or_else(|| {
                refusal(
                    "minsert.block_metadata_missing",
                    format!(
                        "block {:?} has no stored name/base point, so the insertion transform \
                         cannot be built",
                        definition.id
                    ),
                )
            })?;
            (definition.id, metadata.name.clone(), metadata.base_point)
        };

        let to_origin = Transform3::translation(Point3 {
            x: -base_point.x,
            y: -base_point.y,
            z: -base_point.z,
        });
        let mut candidate = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let drawing = &mut candidate;
        let base_order = next_model_draw_order(drawing);
        let mut references: Vec<DbEntity> = Vec::with_capacity(produced as usize);
        let mut index = 0usize;
        for row in 0..request.rows {
            for column in 0..request.columns {
                let cell = Point3 {
                    x: request.insertion.x + column as f64 * request.column_spacing,
                    y: request.insertion.y + row as f64 * request.row_spacing,
                    z: request.insertion.z,
                };
                let transform = Transform3::translation(cell).matrix_mul(&to_origin);
                let id = drawing.allocate_entity_id();
                references.push(DbEntity {
                    object: DbObject {
                        id: ObjectId(id.0),
                        type_key: "AcDbBlockReference".into(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id,
                    layer: session.active_layer,
                    space: SpaceId::Model,
                    geometry: SemanticGeometry::Insert { block, transform },
                    draw_order: base_order + index as i64,
                });
                index += 1;
            }
        }

        let label = "minsert";
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            for reference in &references {
                staged.insert_entity(reference.clone())?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = references
            .iter()
            .map(|reference| drawing_patch(reference.id, None, Some(reference.clone())))
            .collect();
        self.publish_candidate(
            command.document,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
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
            objects: references
                .iter()
                .map(|reference| ObjectId(reference.id.0))
                .collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.minsert".into(),
                message: format!(
                    "MINSERT: block {:?} '{}' inserted as {} x {} independent references ({}, {} \
                     spacing) at ({}, {}, {}); the array is not a single MINSERT entity and is not \
                     associative",
                    block,
                    block_name,
                    request.rows,
                    request.columns,
                    request.row_spacing,
                    request.column_spacing,
                    request.insertion.x,
                    request.insertion.y,
                    request.insertion.z
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
