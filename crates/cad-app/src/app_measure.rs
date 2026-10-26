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
                session.begin_measurement(*kind)?;
                Ok(self.measure_preview_outcome(session))
            }
            CommandPayload::None => {
                // Keep the legacy no-payload command contract explicitly Distance.
                session.begin_measurement(MeasurementToolKind::Distance)?;
                Ok(self.measure_preview_outcome(session))
            }
            CommandPayload::Points(points) => self.capture_measure_points(session, command, points),
            _ => Err(CadError::InvalidInput(
                "Measure needs picked points or a measurement tool".into(),
            )),
        }
    }

    /// Capture points for the active tool, or evaluate the stateless fallback.
    ///
    /// `ID` is refused on this path: it is an **interactive one-pick** command
    /// whose result carries the snap provenance resolved at capture time, and a
    /// `Points` payload has no acquisition layer to resolve one. Accepting it
    /// would either fabricate `snap: null` for a real snap or silently pretend a
    /// payload pick was interactive — so it is refused by name and nothing is
    /// evaluated.
    pub(crate) fn capture_measure_points(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        points: &[Point3],
    ) -> CadResult<CommandOutcome> {
        if let ToolState::Measuring(tool) = &session.tool {
            if tool.kind() == MeasurementToolKind::Id {
                return Err(CadError::InvalidInput(
                    "ID needs an interactive pick; it takes no point payload".into(),
                ));
            }
        }
        let auto_complete = if let ToolState::Measuring(tool) = &session.tool {
            let kind = tool.kind();
            for point in points {
                session.capture_point(*point)?;
            }
            match &session.tool {
                ToolState::Measuring(tool) if tool.auto_ready() => {
                    Some((kind, tool.points().to_vec()))
                }
                _ => None,
            }
        } else {
            None
        };

        if let Some((kind, captured)) = auto_complete {
            let algorithm = kind.algorithm().ok_or_else(|| {
                CadError::InvalidInput(
                    "this tool has no measurement algorithm; it is not evaluated".into(),
                )
            })?;
            let outcome =
                self.evaluate_measurement(&session.active_space, command, algorithm, captured)?;
            if let Some(record) = &outcome.measurement {
                session.set_last_measurement(record.clone());
            }
            // A one-shot tool returns to navigation after a result; open-ended
            // tools stay active until confirmed or cancelled.
            session.complete_tool();
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
                ));
            }
        };
        let outcome =
            self.evaluate_measurement(&session.active_space, command, algorithm, points.to_vec())?;
        if let Some(record) = &outcome.measurement {
            session.set_last_measurement(record.clone());
        }
        Ok(outcome)
    }

    /// Confirm an open-ended measurement tool (polyline/area) and evaluate it, or
    /// confirm `ID` and report the picked coordinate.
    ///
    /// The `ID` branch runs **before** [`Self::evaluate_measurement`]: the
    /// measurement engine has no point algorithm and its record's `value` is a
    /// single `f64`, so an identifier must never be routed through it (see
    /// [`crate::point_report`]).
    pub(crate) fn confirm_measurement(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (kind, points, snap) = match &session.tool {
            ToolState::Measuring(tool) if tool.is_ready() => {
                (tool.kind(), tool.points().to_vec(), tool.snap().cloned())
            }
            ToolState::Measuring(tool) => {
                return Err(CadError::InvalidInput(format!(
                    "measurement needs {} more point(s)",
                    tool.remaining()
                )));
            }
            _ => return Err(CadError::InvalidInput("no measurement in progress".into())),
        };
        if kind == MeasurementToolKind::Id {
            let outcome =
                self.point_report_outcome(command, &session.active_space, &points, snap)?;
            session.complete_tool();
            return Ok(outcome);
        }
        let algorithm = kind.algorithm().ok_or_else(|| {
            CadError::InvalidInput(
                "this tool has no measurement algorithm; it is not evaluated".into(),
            )
        })?;
        let active_space = session.active_space.clone();
        let outcome = self.evaluate_measurement(&active_space, command, algorithm, points)?;
        if let Some(record) = &outcome.measurement {
            session.set_last_measurement(record.clone());
        }
        session.complete_tool();
        Ok(outcome)
    }

    /// Build the `ID` [`PointReport`] for the captured pick.
    ///
    /// Refusals (nothing is recorded, no history/undo step, no database access at
    /// all):
    ///
    /// * `SpaceId::Block` — block-definition space is reached only through an
    ///   `INSERT` and is never a selectable observation space (`Unsupported`);
    /// * a non-plan view — the plan pick mapping (`Viewport::screen_to_world`) is
    ///   only defined for the orthographic top view, so a perspective/3D view
    ///   cannot yield a world point to report (`Unsupported`). The host's capture
    ///   boundary already refuses such a pixel; this is the same fact stated at
    ///   the command layer rather than assumed.
    ///
    /// The unit comes from the open document's own [`UnitContext`]; an unknown
    /// display unit stays `None` instead of being fabricated as millimetres.
    pub(crate) fn point_report_outcome(
        &self,
        command: &Command,
        active_space: &SpaceId,
        points: &[Point3],
        snap: Option<crate::point_report::InputSnap>,
    ) -> CadResult<CommandOutcome> {
        let space = IdSpace::from_space_id(active_space).ok_or_else(|| {
            CadError::Unsupported(
                "ID cannot report a block-definition space point; switch to model or paper space"
                    .into(),
            )
        })?;
        let position = points.last().copied().ok_or_else(|| {
            CadError::InvalidInput("ID needs one captured point before it can report".into())
        })?;
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
        if !viewport.camera.projection.is_orthographic() {
            return Err(CadError::Unsupported(
                "ID needs the plan (orthographic top) view: a perspective view has no single world \
                 point under the cursor"
                    .into(),
            ));
        }
        let report = PointReport {
            position,
            space,
            snap: snap.as_ref().map(|snap| snap.kind),
            // The report's precision is the snapped geometry's when a snap
            // resolved; a raw pick has no approximated source.
            precision: snap
                .as_ref()
                .map_or(Precision::Analytic, |snap| snap.precision.clone()),
            unit: crate::point_report::unit_symbol(&document.units).map(str::to_string),
            logical_pixel_distance: snap.as_ref().map(|snap| snap.logical_pixel_distance),
        };
        let message = match report.snap {
            Some(kind) => format!(
                "ID: {:.6}, {:.6}, {:.6} ({} snap, {})",
                report.position.x,
                report.position.y,
                report.position.z,
                kind.as_str(),
                report.space.as_str()
            ),
            None => format!(
                "ID: {:.6}, {:.6}, {:.6} ({})",
                report.position.x,
                report.position.y,
                report.position.z,
                report.space.as_str()
            ),
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "measure.id".into(),
                message,
            }],
            measurement: None,
            point_report: Some(report),
        })
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
            point_report: None,
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
            point_report: None,
        })
    }
}
