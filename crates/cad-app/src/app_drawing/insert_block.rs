//! `INSERT`: create a block reference from an **existing** block definition.
//!
//! This is the bounded v1 creation side: the block table itself is read-only
//! (`BLOCK`/`WBLOCK` are not implemented). The command resolves the block by
//! name against the drawing's stored name/base-point metadata and inserts one
//! `SemanticGeometry::Insert` through the shared typed create funnel (one
//! transaction, one undo step, active layer, model space).
//!
//! The transform follows the importer's own convention exactly: a reference's
//! transform is `T(insertion) · T(−base_point)`, so the block point
//! `base_point` lands on `insertion` (`transform.apply_point(base_point) ==
//! insertion`).
//!
//! Refusals happen **before** any transaction and change nothing:
//! `insert.invalid_point`, `insert.invalid_name`, `insert.unknown_block` and
//! `insert.block_metadata_missing` (a block with no stored base point is never
//! given an invented zero base).

use super::*;

impl Application {
    /// Create one block reference at the picked insertion point.
    pub(crate) fn insert_block(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::InsertBlock { insertion, name } = &command.payload else {
            return Err(CadError::InvalidInput(
                "INSERT needs an InsertBlock { insertion, name } payload".into(),
            ));
        };
        if !is_finite_point(*insertion) {
            return Err(CadError::InvalidInput(
                "insert.invalid_point: INSERT insertion point must be finite".into(),
            ));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(CadError::InvalidInput(
                "insert.invalid_name: INSERT needs a non-empty block name".into(),
            ));
        }
        if name.contains('\r') || name.contains('\n') {
            return Err(CadError::InvalidInput(
                "insert.invalid_name: INSERT block name must be a single line".into(),
            ));
        }
        let characters = name.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "insert.invalid_name: INSERT allows at most {MAX_TOOL_TEXT_CHARS} characters, \
                 got {characters}"
            )));
        }

        // Resolve the definition and its metadata from the authoritative drawing
        // and release the borrow before the create funnel mutates the document.
        let (block, block_name, base_point) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            let definition = drawing.block_by_name(name).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "insert.unknown_block: no block named '{name}' in this drawing"
                ))
            })?;
            let metadata = drawing.block_metadata(definition.id).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "insert.block_metadata_missing: block {:?} has no stored name/base point",
                    definition.id
                ))
            })?;
            (definition.id, metadata.name.clone(), metadata.base_point)
        };

        // The importer stores `T(insertion) · T(−base_point)`; a new reference
        // must use the same composition so expansion lands the base on the click.
        let to_origin = Transform3::translation(Point3 {
            x: -base_point.x,
            y: -base_point.y,
            z: -base_point.z,
        });
        let transform = Transform3::translation(*insertion).matrix_mul(&to_origin);
        let geometry = SemanticGeometry::Insert { block, transform };
        let mut outcome = self.commit_new_entity_typed(
            session,
            command,
            geometry,
            "AcDbBlockReference",
            "insert block",
        )?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "drawing.insert".into(),
            message: format!(
                "INSERT: block {:?} '{}' placed at ({}, {}, {})",
                block, block_name, insertion.x, insertion.y, insertion.z
            ),
        });
        Ok(outcome)
    }
}
