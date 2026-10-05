//! Measurement commands for [`Application`].
use super::*;

impl Application {
    /// Dispatch the measurement tool (spec F06/U04).
    ///
    /// The algorithm comes from the active tool kind, never from the raw point
    /// count. A stateless `Points` payload with no active tool keeps the old
    /// convenience inference for direct callers and CLI; the interactive tool
    /// path is the one the UI uses.
    pub(crate) fn measure(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::MeasureTool(kind) => {
                session.tool = ToolState::Measuring(MeasurementTool::new(*kind));
                Ok(self.measure_preview_outcome(session))
            }
            CommandPayload::None => {
                // The shell currently sends no algorithm; default to distance.
                session.tool =
                    ToolState::Measuring(MeasurementTool::new(MeasurementToolKind::Distance));
                Ok(self.measure_preview_outcome(session))
            }
            CommandPayload::Points(points) => self.capture_measure_points(session, command, points),
            _ => Err(CadError::InvalidInput(
                "Measure needs picked points or a measurement tool".into(),
            )),
        }
    }

    /// Capture points for the active tool, or evaluate the stateless fallback.
    pub(crate) fn capture_measure_points(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        points: &[Point3],
    ) -> CadResult<CommandOutcome> {
        let auto_complete = if let ToolState::Measuring(tool) = &mut session.tool {
            for point in points {
                tool.push_point(*point);
            }
            if tool.auto_ready() {
                Some((tool.kind(), tool.points().to_vec()))
            } else {
                None
            }
        } else {
            None
        };

        if let Some((kind, captured)) = auto_complete {
            let outcome = self.evaluate_measurement(
                &session.active_space,
                command,
                kind.algorithm(),
                captured,
            )?;
            if let Some(record) = &outcome.measurement {
                session.set_last_measurement(record.clone());
            }
            // A one-shot tool returns to navigation after a result; open-ended
            // tools stay active until confirmed or cancelled.
            session.tool = ToolState::Idle;
            return Ok(outcome);
        }
        if matches!(session.tool, ToolState::Measuring(_)) {
            return Ok(self.measure_preview_outcome(session));
        }

        // No active tool: stateless inference retained for direct callers/CLI.
        let algorithm = match points.len() {
            2 => MeasurementAlgorithm::Distance3d,
            3 => MeasurementAlgorithm::Angle3Points,
            n if n >= 4 => MeasurementAlgorithm::PolylineLength,
            _ => {
                return Err(CadError::InvalidInput(
                    "measurement needs two (distance), three (angle) or more (length) points"
                        .into(),
                ))
            }
        };
        let outcome =
            self.evaluate_measurement(&session.active_space, command, algorithm, points.to_vec())?;
        if let Some(record) = &outcome.measurement {
            session.set_last_measurement(record.clone());
        }
        Ok(outcome)
    }

    /// Confirm an open-ended measurement tool (polyline/area) and evaluate it.
    pub(crate) fn confirm_measurement(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (kind, points) = match &session.tool {
            ToolState::Measuring(tool) if tool.is_ready() => (tool.kind(), tool.points().to_vec()),
            ToolState::Measuring(tool) => {
                return Err(CadError::InvalidInput(format!(
                    "measurement needs {} more point(s)",
                    tool.remaining()
                )))
            }
            _ => return Err(CadError::InvalidInput("no measurement in progress".into())),
        };
        let active_space = session.active_space.clone();
        let outcome =
            self.evaluate_measurement(&active_space, command, kind.algorithm(), points)?;
        if let Some(record) = &outcome.measurement {
            session.set_last_measurement(record.clone());
        }
        session.tool = ToolState::Idle;
        Ok(outcome)
    }

    pub(crate) fn measure_preview_outcome(&self, session: &SessionState) -> CommandOutcome {
        let Some(preview) = session.measurement_preview() else {
            return CommandOutcome::none();
        };
        let message = preview.status_line();
        CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "measure.preview".into(),
                message,
            }],
            measurement: None,
        }
    }

    /// Evaluate a measurement through the engine and return the structured
    /// record. The space comes from the session's active space (F04) and the
    /// algorithm from the tool; area uses the viewport work plane so non-coplanar
    /// input is rejected rather than silently flattened (audit B24).
    pub(crate) fn evaluate_measurement(
        &self,
        active_space: &SpaceId,
        command: &Command,
        algorithm: MeasurementAlgorithm,
        points: Vec<Point3>,
    ) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let viewport = self
            .workspace
            .viewports
            .get(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let (algorithm, space) =
            resolve_measurement_space(active_space, viewport.work_plane, algorithm)?;
        let request = MeasurementRequest {
            algorithm,
            points,
            tapped: Vec::new(),
            space,
            units: document.units.clone(),
            source: GeometrySource::UserPoints,
            precision: Precision::Analytic,
        };
        let record = self.measurement.measure(&request)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "measure.result".into(),
                message: format!("{:?}: {:.6}", record.algorithm, record.value),
            }],
            measurement: Some(record),
        })
    }
}
