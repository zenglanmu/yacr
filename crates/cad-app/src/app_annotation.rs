//! Annotation commands for [`Application`].
use super::*;

impl Application {
    pub(crate) fn annotation_command(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let payload = match &command.payload {
            CommandPayload::Annotation(annotation_command) => annotation_command.as_ref(),
            _ => {
                return Err(CadError::InvalidInput(
                    "annotation command needs an annotation payload".into(),
                ))
            }
        };
        self.commit_annotation(document_id, payload)
    }

    /// Start (or restart) an annotation creation tool (spec F07/U04).
    pub(crate) fn begin_annotation_tool(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::AnnotationTool(kind) = &command.payload else {
            return Err(CadError::InvalidInput(
                "BeginAnnotationTool needs an annotation kind payload".into(),
            ));
        };
        session.tool = ToolState::Annotating(AnnotationTool::new(*kind));
        Ok(self.annotation_preview_outcome(session))
    }

    /// Capture world points / text into the active annotation tool.
    ///
    /// Fixed-point kinds capture the exact complement and then return; open
    /// ended kinds accumulate until an explicit confirm. A stray point with no
    /// active tool is refused rather than silently ignored.
    pub(crate) fn capture_annotation_points(
        &mut self,
        session: &mut SessionState,
        points: &[Point3],
    ) -> CadResult<CommandOutcome> {
        let outcome = match &mut session.tool {
            ToolState::Annotating(tool) => {
                for point in points {
                    tool.push_point(*point)?;
                }
                if tool.auto_ready() {
                    Some(self.build_annotation_outcome(session)?)
                } else {
                    None
                }
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "no annotation tool is active".into(),
                ))
            }
        };
        if let Some(outcome) = outcome {
            // A one-shot tool returns to navigation after committing.
            session.tool = ToolState::Idle;
            return Ok(outcome);
        }
        Ok(self.annotation_preview_outcome(session))
    }

    /// Confirm an annotation tool: build and commit exactly one transaction.
    ///
    /// The tool is cleared only after the transaction has been applied, so a
    /// rejected commit leaves the captured parameters in place for the user to
    /// fix (mirroring the measurement tool).
    pub(crate) fn confirm_annotation_tool(
        &mut self,
        session: &mut SessionState,
    ) -> CadResult<CommandOutcome> {
        let outcome = self.build_annotation_outcome(session)?;
        session.tool = ToolState::Idle;
        Ok(outcome)
    }

    /// Build and commit the annotation described by the active tool.
    pub(crate) fn build_annotation_outcome(
        &mut self,
        session: &SessionState,
    ) -> CadResult<CommandOutcome> {
        let (annotation, document_id) = {
            let tool = match &session.tool {
                ToolState::Annotating(tool) => tool,
                _ => {
                    return Err(CadError::InvalidInput(
                        "no annotation tool is active".into(),
                    ))
                }
            };
            let id = self.next_annotation_id(session.document);
            let annotation = tool.build(
                id,
                session.active_space.clone(),
                0,
                0,
                AnnotationStyle::default(),
            )?;
            (annotation, session.document)
        };
        let command = AnnotationCommand::Create(annotation);
        self.commit_annotation(document_id, &command)
    }

    /// Allocate the next annotation id in the open document's id space.
    ///
    /// Deterministic and monotonic: one past the highest existing id, so a
    /// delete never lets a later create reuse an id (which would make a stale
    /// visibility override or selection point at the wrong annotation). A real
    /// host may prefer UUIDs, but id allocation is not the tool's concern and a
    /// stable sequence keeps the command path reproducible for tests and the CLI.
    pub(crate) fn next_annotation_id(&self, document: DocumentId) -> AnnotationId {
        let next = self
            .workspace
            .documents
            .get(&document)
            .map(|d| {
                d.annotations
                    .annotations()
                    .map(|a| a.id.0)
                    .max()
                    .unwrap_or(0)
                    + 1
            })
            .unwrap_or(1);
        AnnotationId(next)
    }

    pub(crate) fn annotation_preview_outcome(&self, session: &SessionState) -> CommandOutcome {
        let Some(preview) = session.annotation_preview() else {
            return CommandOutcome::none();
        };
        CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "annotation.preview".into(),
                message: preview.status_line(),
            }],
            measurement: None,
            annotation: None,
        }
    }

    /// Commit one annotation command through the shared history path.
    ///
    /// This is the single write entry used both by the tool confirm and by the
    /// management UI's delete/edit actions, so there is exactly one transaction
    /// and one undo record per user action. Any failure leaves the database,
    /// its revision and history untouched (the service rolls the transaction
    /// back before this function records anything).
    pub(crate) fn commit_annotation(
        &mut self,
        document_id: DocumentId,
        command: &AnnotationCommand,
    ) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

        // Build the undo patch from the before/after state.
        let (patch, label) = match command {
            AnnotationCommand::Create(a) => {
                (patch(a.id, None, Some(a.clone())), "create annotation")
            }
            AnnotationCommand::Update(a) => {
                let before = document.annotations.get(a.id).cloned();
                (patch(a.id, before, Some(a.clone())), "update annotation")
            }
            AnnotationCommand::Delete(id) => {
                let before = document.annotations.get(*id).cloned();
                (patch(*id, before, None), "delete annotation")
            }
        };
        let annotation = match command {
            AnnotationCommand::Create(a) | AnnotationCommand::Update(a) => a.id,
            AnnotationCommand::Delete(id) => *id,
        };
        let inner = match command {
            AnnotationCommand::Create(a) => AnnotationCommand::Create(a.clone()),
            AnnotationCommand::Update(a) => AnnotationCommand::Update(a.clone()),
            AnnotationCommand::Delete(id) => AnnotationCommand::Delete(*id),
        };
        let changes = self.annotations.apply(&mut document.annotations, inner)?;

        let history = self.history.entry(document_id).or_default();
        let transaction = changes.transaction;
        history.record(UndoRecord {
            transaction,
            label: label.to_string(),
            patches: vec![patch],
            merge_key: None,
        })?;

        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(changes),
            diagnostics: Vec::new(),
            measurement: None,
            annotation: Some(annotation),
        })
    }
}
