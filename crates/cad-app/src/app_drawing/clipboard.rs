use super::*;

impl Application {
    /// Capture the current selection into the in-process clipboard.
    ///
    /// Read-only: it clones the selected entities and opens **no** transaction,
    /// so the drawing revision and undo history are untouched. Allowed in Viewer
    /// mode. An empty selection is an explicit refusal, never an empty success.
    pub(crate) fn copy_to_clipboard(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (entries, mut diagnostics) = self.capture_selection_to_clipboard(session, command)?;
        let count = entries.len();
        session.clipboard = entries;
        diagnostics.insert(
            0,
            Diagnostic {
                object: None,
                code: "drawing.clipboard.copy".into(),
                message: format!("已复制 {count} 个实体到剪贴板"),
            },
        );
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// Capture the current selection into the clipboard, then delete it.
    ///
    /// The capture is read-only; the delete is one transaction and one undo step
    /// (the same validated delete/patch path ERASE uses, so render attributes and
    /// dynamic-visibility membership are restored on undo). An empty selection is
    /// refused before anything is captured or deleted.
    pub(crate) fn cut_to_clipboard(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (entries, diagnostics) = self.capture_selection_to_clipboard(session, command)?;
        let count = entries.len();
        let deleted = self.erase_entities(session, command)?;
        // Commit the new clipboard only after the delete succeeded, so a failed
        // cut cannot destroy the previous clipboard.
        session.clipboard = entries;
        let mut merged = vec![Diagnostic {
            object: None,
            code: "drawing.clipboard.cut".into(),
            message: format!("已剪切 {count} 个实体（已删除并移入剪贴板）"),
        }];
        merged.extend(diagnostics);
        merged.extend(deleted.diagnostics);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: deleted.changes,
            diagnostics: merged,
            measurement: None,
            point_report: None,
        })
    }

    /// Insert fresh-id clones of the clipboard entities at their original
    /// coordinates (PASTEORIG semantics) in one transaction and one undo step.
    ///
    /// The clipboard is the in-process one (never the OS clipboard). An empty
    /// clipboard is an explicit refusal. The whole paste is validated atomically
    /// by the transaction, so invalid clipboard content refuses the entire paste
    /// rather than inserting a partial set. Source layer/space are preserved, and
    /// each copy's captured display state (render attributes + dynamic-visibility
    /// membership) is restored, so a paste matches what a cut's undo restores.
    pub(crate) fn paste_from_clipboard(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "PasteFromClipboard takes no payload".into(),
            ));
        }
        if session.clipboard.is_empty() {
            return Err(CadError::InvalidInput("clipboard is empty".into()));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        let base_order = drawing
            .model_space()
            .iter()
            .map(|e| e.draw_order)
            .max()
            .map(|m| m.saturating_add(1))
            .unwrap_or(0);
        let mut copies: Vec<(EntityId, DbEntity)> = Vec::with_capacity(session.clipboard.len());
        for (index, entry) in session.clipboard.iter().enumerate() {
            let source = &entry.entity;
            let new_id = drawing.allocate_entity_id();
            let copy = DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(new_id.0),
                    type_key: source.object.type_key.clone(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                layer: source.layer,
                space: source.space.clone(),
                geometry: source.geometry.clone(),
                draw_order: base_order + index as i64,
            };
            copies.push((new_id, copy));
        }

        // The transaction validates every insert before mutating anything, so a
        // bad clipboard entry refuses the whole paste (never a partial insert).
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("paste clipboard", transaction)?;
            for (_, copy) in &copies {
                staged.insert_entity(copy.clone())?;
            }
            staged.commit()?
        };
        // Restore the captured display state for every pasted copy, so a paste
        // preserves exactly what a cut's undo restores (render attributes and
        // dynamic-visibility membership); record it on the patch so a redo
        // re-applies it too.
        for (index, (id, _)) in copies.iter().enumerate() {
            drawing.restore_entity_display_state(*id, &session.clipboard[index].display);
        }
        let patches: Vec<DrawingPatch> = copies
            .iter()
            .enumerate()
            .map(|(index, (id, copy))| DrawingPatch {
                id: *id,
                before: None,
                after: Some(copy.clone()),
                before_state: None,
                after_state: Some(session.clipboard[index].display.clone()),
            })
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "paste clipboard".into(),
                patches,
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: copies.iter().map(|(id, _)| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.clipboard.paste".into(),
                message: format!("从剪贴板粘贴 {} 个实体（原坐标）", copies.len()),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// Resolve the current selection into clipboard entries (read-only snapshot).
    ///
    /// Collapses duplicate refs, requires a non-empty selection, and captures each
    /// entity together with its [`cad_db::EntityDisplayState`] (read before any
    /// cut deletes it). Sub-element/instance refs carry capture-specific
    /// diagnostics: a read-only capture affects no other instance. Returns the
    /// entries and diagnostics but does **not** touch `session.clipboard`, so a
    /// caller can commit them only once its own precondition (e.g. a successful
    /// delete) holds.
    fn capture_selection_to_clipboard(
        &self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<(Vec<ClipboardEntry>, Vec<Diagnostic>)> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "clipboard capture takes no payload".into(),
            ));
        }
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "clipboard capture needs a non-empty selection".into(),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let (entities, diagnostics) = Self::resolve_selection(
            &document.drawing,
            document_id,
            &refs,
            "clipboard",
            "Clipboard",
            "复制",
            true,
        )?;
        let entries: Vec<ClipboardEntry> = entities
            .into_iter()
            .map(|entity| {
                let display = document.drawing.entity_display_state(entity.id);
                ClipboardEntry { entity, display }
            })
            .collect();
        Ok((entries, diagnostics))
    }
}
