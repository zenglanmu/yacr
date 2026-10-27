use super::*;

/// The clipboard command cluster (`COPYCLIP`/`CUTCLIP`/`COPYBASE`/`CUTBASE`/
/// `PASTECLIP`/`PASTEORIG`) over the **in-process** clipboard.
///
/// The clipboard is session state ([`SessionState::clipboard`] +
/// [`SessionState::clipboard_base`]), never the OS clipboard, and it is cleared
/// when the document content is replaced. The two capture shapes and the two
/// paste shapes are deliberately separate, AutoCAD-faithful commands:
///
/// * `COPYCLIP` / `CUTCLIP` — the session selection, **no** base point;
/// * `COPYBASE` / `CUTBASE` — the payload's refs plus a captured **base point**;
/// * `PASTECLIP` — fresh-id clones translated by `insertion − base`, where a
///   clipboard without a base (the `*CLIP` pair) uses the world origin;
/// * `PASTEORIG` — fresh-id clones at their **original** coordinates (the base is
///   ignored): the behaviour `PASTECLIP` had before it became insertion-point
///   based, kept reachable under its own AutoCAD name.
impl Application {
    /// Capture the current selection into the in-process clipboard.
    ///
    /// Read-only: it clones the selected entities and opens **no** transaction,
    /// so the drawing revision and undo history are untouched. Allowed in Viewer
    /// mode. An empty selection is an explicit refusal, never an empty success.
    /// `COPYCLIP` records **no** base point, so a later `PASTECLIP` translates
    /// relative to the world origin (AutoCAD's documented behaviour for a
    /// non-`COPYBASE` clipboard).
    pub(crate) fn copy_to_clipboard(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (entries, mut diagnostics) = self.capture_selection_to_clipboard(session, command)?;
        let count = entries.len();
        session.clipboard = entries;
        // `COPYCLIP` carries no base point: a stale base from an earlier
        // `COPYBASE` must not redirect this capture's pastes.
        session.clipboard_base = None;
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

    /// `COPYBASE`: capture the payload's refs into the clipboard **with** the
    /// captured base point.
    ///
    /// Read-only (no transaction, no history, no revision bump), so Viewer mode may
    /// run it. The refs are the selection snapshot taken when the
    /// [`crate::draw_tool::DrawToolKind::CopyBase`] capture started; the base is the
    /// one captured point. The clipboard records `Some(base)` so `PASTECLIP` can
    /// translate by `insertion − base`. An empty refs list or a non-finite base is
    /// an explicit refusal that leaves the previous clipboard untouched.
    pub(crate) fn copy_to_clipboard_base(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::CopyBase { refs, base } = &command.payload else {
            return Err(CadError::InvalidInput(
                "COPYBASE needs a CopyBase { refs, base } payload".into(),
            ));
        };
        if !is_finite_point(*base) {
            return Err(CadError::InvalidInput(
                "COPYBASE base point must be finite".into(),
            ));
        }
        let (entries, mut diagnostics) =
            self.capture_refs_to_clipboard(command.document, refs, "复制")?;
        let count = entries.len();
        session.clipboard = entries;
        session.clipboard_base = Some(*base);
        diagnostics.insert(
            0,
            Diagnostic {
                object: None,
                code: "drawing.clipboard.copybase".into(),
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
    /// refused before anything is captured or deleted. `CUTCLIP` records **no**
    /// base point (see [`Self::copy_to_clipboard`]).
    pub(crate) fn cut_to_clipboard(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (entries, diagnostics) = self.capture_selection_to_clipboard(session, command)?;
        let count = entries.len();
        let refs = session.selection.refs().to_vec();
        let (changes, deleted_diagnostics) =
            self.delete_entity_refs(command.document, &refs, "CUT")?;
        // Commit the new clipboard only after the delete succeeded, so a failed
        // cut cannot destroy the previous clipboard.
        session.clipboard = entries;
        session.clipboard_base = None;
        let mut merged = vec![Diagnostic {
            object: None,
            code: "drawing.clipboard.cut".into(),
            message: format!("已剪切 {count} 个实体（已删除并移入剪贴板）"),
        }];
        merged.extend(diagnostics);
        merged.extend(deleted_diagnostics);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics: merged,
            measurement: None,
            point_report: None,
        })
    }

    /// `CUTBASE`: [`Self::copy_to_clipboard_base`] **and** delete exactly the
    /// payload refs.
    ///
    /// The capture is read-only and happens first; the delete is the shared
    /// validated delete path in one transaction / one undo step, driven by the
    /// **payload refs** (never the session selection, which need not match the
    /// snapshot the capture started with). The clipboard — entries **and** base —
    /// is committed only after the delete succeeded, so a refused cut leaves the
    /// previous clipboard intact. Work-only (the command's `requires_work_mode`).
    pub(crate) fn cut_to_clipboard_base(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::CutBase { refs, base } = &command.payload else {
            return Err(CadError::InvalidInput(
                "CUTBASE needs a CutBase { refs, base } payload".into(),
            ));
        };
        if !is_finite_point(*base) {
            return Err(CadError::InvalidInput(
                "CUTBASE base point must be finite".into(),
            ));
        }
        let (entries, diagnostics) =
            self.capture_refs_to_clipboard(command.document, refs, "复制")?;
        let count = entries.len();
        let (changes, deleted_diagnostics) =
            self.delete_entity_refs(command.document, refs, "CUT")?;
        // Only a successful delete may replace the clipboard, base included.
        session.clipboard = entries;
        session.clipboard_base = Some(*base);
        let mut merged = vec![Diagnostic {
            object: None,
            code: "drawing.clipboard.cutbase".into(),
            message: format!("已剪切 {count} 个实体（已删除并移入剪贴板）"),
        }];
        merged.extend(diagnostics);
        merged.extend(deleted_diagnostics);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics: merged,
            measurement: None,
            point_report: None,
        })
    }

    /// `PASTECLIP`: insert fresh-id clones of the clipboard entities translated by
    /// `insertion − clipboard base`, in one transaction and one undo step.
    ///
    /// AutoCAD's documented insertion-point paste, and the reason `COPYBASE`
    /// exists: `base` is [`SessionState::clipboard_base`] and the **world origin**
    /// when the clipboard has none (`COPYCLIP`/`CUTCLIP`). An empty clipboard, a
    /// non-finite insertion point or a clipboard entry whose geometry cannot be
    /// translated refuses the whole paste before a write (never a partial insert).
    /// Source layer/space are preserved and each copy's captured display state is
    /// restored, exactly like [`Self::paste_original_coordinates`].
    pub(crate) fn paste_from_clipboard(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::PasteClip { insertion } = command.payload else {
            return Err(CadError::InvalidInput(
                "PASTECLIP needs a PasteClip { insertion } payload".into(),
            ));
        };
        if !is_finite_point(insertion) {
            return Err(CadError::InvalidInput(
                "PASTECLIP insertion point must be finite".into(),
            ));
        }
        let base = session.clipboard_base.unwrap_or(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        // The one translation rule of the insertion-point paste.
        let delta = Point3 {
            x: insertion.x - base.x,
            y: insertion.y - base.y,
            z: insertion.z - base.z,
        };
        if !is_finite_point(delta) {
            return Err(CadError::InvalidInput(
                "PASTECLIP translation must be finite".into(),
            ));
        }
        self.paste_clipboard(session, command, Some(delta), |count| {
            format!("pasted {count} entities from the clipboard at the insertion point")
        })
    }

    /// `PASTEORIG`: insert fresh-id clones at their **original** coordinates.
    ///
    /// The clipboard base is deliberately ignored (that is what makes this
    /// command distinct from `PASTECLIP`), and no geometry transform runs at all,
    /// so the result is byte-for-byte the pre-`PASTECLIP` behaviour. One
    /// transaction / one undo step; an empty clipboard is an explicit refusal.
    pub(crate) fn paste_original_coordinates(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput("PASTEORIG takes no payload".into()));
        }
        self.paste_clipboard(session, command, None, |count| {
            format!("从剪贴板粘贴 {} 个实体（原坐标）", count)
        })
    }

    /// The shared paste body: fresh-id clones of the clipboard, optionally
    /// translated by `delta`, in one transaction / one undo step.
    ///
    /// `delta = None` copies each entry's geometry verbatim (the `PASTEORIG`
    /// path). `delta = Some(d)` runs the same [`cad_db::transform_geometry`]
    /// translation COPY uses, so there is exactly one translation rule in the
    /// build; a geometry that cannot be translated refuses the whole paste before
    /// the transaction opens.
    fn paste_clipboard(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        delta: Option<Point3>,
        message: impl Fn(usize) -> String,
    ) -> CadResult<CommandOutcome> {
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
        let translation = delta.filter(|d| {
            *d != Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }
        });
        let mut copies: Vec<(EntityId, DbEntity)> = Vec::with_capacity(session.clipboard.len());
        for (index, entry) in session.clipboard.iter().enumerate() {
            let source = &entry.entity;
            let new_id = drawing.allocate_entity_id();
            let geometry = match translation {
                Some(delta) => {
                    transform_geometry(&source.geometry, &Transform3::translation(delta)).map_err(
                        |error| {
                            CadError::Unsupported(format!(
                                "PASTE cannot transform {}: {error}",
                                geometry_kind(&source.geometry)
                            ))
                        },
                    )?
                }
                // No translation (PASTEORIG, or an insertion point equal to the
                // base): the geometry is copied verbatim, so no transform rule can
                // alter or refuse it.
                None => source.geometry.clone(),
            };
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
                geometry,
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
                message: message(copies.len()),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// Resolve the **session selection** into clipboard entries (the
    /// `COPYCLIP`/`CUTCLIP` shape, which takes no payload and records no base).
    ///
    /// Returns the entries and diagnostics but does **not** touch
    /// `session.clipboard`, so a caller can commit them only once its own
    /// precondition (e.g. a successful delete) holds.
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
        self.capture_refs_to_clipboard(command.document, session.selection.refs(), "复制")
    }

    /// Resolve `refs` into clipboard entries (read-only snapshot).
    ///
    /// Collapses duplicate refs, requires a non-empty list, and captures each
    /// entity together with its [`cad_db::EntityDisplayState`] (read before any
    /// cut deletes it). Sub-element/instance refs carry capture-specific
    /// diagnostics: a read-only capture affects no other instance. Returns the
    /// entries and diagnostics but does **not** touch `session.clipboard`, so a
    /// caller can commit them only once its own precondition (e.g. a successful
    /// delete) holds — the `COPYBASE`/`CUTBASE` shape shares this body with the
    /// selection-based `*CLIP` pair, so there is exactly one capture path.
    fn capture_refs_to_clipboard(
        &self,
        document_id: DocumentId,
        refs: &[SelectionRef],
        action: &str,
    ) -> CadResult<(Vec<ClipboardEntry>, Vec<Diagnostic>)> {
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "clipboard capture needs a non-empty selection".into(),
            ));
        }
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let (entities, diagnostics) = Self::resolve_selection(
            &document.drawing,
            document_id,
            refs,
            "clipboard",
            "Clipboard",
            action,
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
