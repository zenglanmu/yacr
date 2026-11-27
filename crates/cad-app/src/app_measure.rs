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
                    Some((kind, tool.points().to_vec(), tool.snaps().to_vec()))
                }
                _ => None,
            }
        } else {
            None
        };

        if let Some((kind, captured, snaps)) = auto_complete {
            let algorithm = kind.algorithm().ok_or_else(|| {
                CadError::InvalidInput(
                    "this tool has no measurement algorithm; it is not evaluated".into(),
                )
            })?;
            let outcome = self.evaluate_measurement(
                &session.active_space,
                command,
                algorithm,
                kind.engine_points(captured),
                &snaps,
            )?;
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
        // A `Points` payload has no acquisition layer, so there is no snap
        // provenance to attribute to any of its points.
        let outcome = self.evaluate_measurement(
            &session.active_space,
            command,
            algorithm,
            points.to_vec(),
            &[],
        )?;
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
        let (kind, points, snap, snaps) = match &session.tool {
            ToolState::Measuring(tool) if tool.is_ready() => (
                tool.kind(),
                tool.points().to_vec(),
                tool.snap().cloned(),
                tool.snaps().to_vec(),
            ),
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
        let outcome = self.evaluate_measurement(
            &active_space,
            command,
            algorithm,
            kind.engine_points(points),
            &snaps,
        )?;
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
        let display = document.drawing.units_display();
        let precision = document.units.decimal_places;
        let x = display.format_length(report.position.x, precision);
        let y = display.format_length(report.position.y, precision);
        let z = display.format_length(report.position.z, precision);
        let message = match report.snap {
            Some(kind) => format!(
                "ID: {x}, {y}, {z} ({} snap, {})",
                kind.as_str(),
                report.space.as_str()
            ),
            None => format!("ID: {x}, {y}, {z} ({})", report.space.as_str()),
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
    ///
    /// ## Provenance summary rule
    ///
    /// `snaps` is positionally parallel to `points` (one entry per capture,
    /// `None` for a raw pick); it may be empty for a stateless `Points` payload,
    /// which has no acquisition layer at all. A non-empty list of a different
    /// length is refused with `measurement.snaps_not_parallel`, so a shifted
    /// parallel array can never attribute one pick's provenance to another.
    ///
    /// * **No snapped point** → `UserPoints` / `Analytic`, the unchanged legacy
    ///   answer: the value comes from coordinates the user supplied, and a raw
    ///   pick has no approximated source.
    /// * **At least one snapped point** → the record names the **least
    ///   authoritative** geometry any snapped point depended on:
    ///   `DirectMesh` if any snapped point came from a display/direct mesh, else
    ///   `ProxyCache` if any came from a vendor proxy cache, else `KernelMesh`
    ///   if any came from an exact kernel mesh. When every snapped point is
    ///   analytic, the presence of **any unsnapped (raw) input point** makes the
    ///   measurement `UserPoints`; only an all-analytic, all-snapped input is
    ///   reported as `Analytic`.
    /// * The precision is the **weakest observed** among the snapped points:
    ///   `Unknown` is weaker than `Approximate`, which is weaker than `Analytic`.
    ///   A raw pick makes no precision claim: it neither strengthens nor weakens
    ///   the snapped points' statement. Two `Approximate` values resolve to the
    ///   one with the larger error bound (`None` = unknown bound, the weakest).
    ///
    /// The record has one `source`/`precision` pair for the whole measurement, so
    /// this is a summary, not a per-point claim. [`CommandOutcome`] carries only
    /// this scalar record: per-point `SnapProvenance` is **not** exposed beside
    /// it (the tool session keeps its own capture-layer snaps).
    pub(crate) fn evaluate_measurement(
        &self,
        active_space: &SpaceId,
        command: &Command,
        algorithm: MeasurementAlgorithm,
        points: Vec<Point3>,
        snaps: &[Option<InputSnap>],
    ) -> CadResult<CommandOutcome> {
        if !snaps.is_empty() && snaps.len() != points.len() {
            return Err(CadError::InvalidInput(format!(
                "measurement.snaps_not_parallel: {} snap entries for {} points",
                snaps.len(),
                points.len()
            )));
        }
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
        let (source, precision) = measurement_provenance(snaps);
        let request = MeasurementRequest {
            algorithm,
            points,
            tapped: Vec::new(),
            space,
            units: document.units.clone(),
            source,
            precision,
        };
        let record = self.measurement.measure(&request)?;
        let value = match record.algorithm {
            MeasurementAlgorithm::Angle3Points => document
                .drawing
                .units_display()
                .format_angle(record.value, false),
            _ => document
                .drawing
                .units_display()
                .format_length(record.value, document.units.decimal_places),
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "measure.result".into(),
                message: format!("{:?}: {value}", record.algorithm),
            }],
            measurement: Some(record),
            point_report: None,
        })
    }
}

/// Summarise per-point snap provenance into the record's single source and
/// precision (see [`Application::evaluate_measurement`] for the documented rule).
fn measurement_provenance(snaps: &[Option<InputSnap>]) -> (GeometrySource, Precision) {
    let snapped: Vec<&InputSnap> = snaps.iter().flatten().collect();
    if snapped.is_empty() {
        // No object snap resolved (or no acquisition layer at all): the legacy
        // user-point answer, never a fabricated mesh provenance.
        return (GeometrySource::UserPoints, Precision::Analytic);
    }
    let has = |source: GeometrySource| snapped.iter().any(|snap| snap.geometry_source == source);
    // The least authoritative source wins: a direct display mesh is weaker
    // evidence than a vendor proxy cache, which is weaker than an exact kernel
    // mesh, because the record may not claim more fidelity than its worst input.
    let source = if has(GeometrySource::DirectMesh) {
        GeometrySource::DirectMesh
    } else if has(GeometrySource::ProxyCache) {
        GeometrySource::ProxyCache
    } else if has(GeometrySource::KernelMesh) {
        GeometrySource::KernelMesh
    } else if snaps.iter().any(Option::is_none) {
        // Every snapped point is analytic, but at least one input point was a
        // raw pick: the value is not derived purely from object geometry.
        GeometrySource::UserPoints
    } else {
        GeometrySource::Analytic
    };
    // A raw pick contributes no precision claim; only the snapped points can
    // weaken the record.
    let precision = snapped
        .into_iter()
        .map(|snap| &snap.precision)
        .reduce(weaker_precision)
        .cloned()
        .unwrap_or(Precision::Analytic);
    (source, precision)
}

/// The weaker of two precisions (`Unknown` < `Approximate` < `Analytic`).
fn weaker_precision<'a>(left: &'a Precision, right: &'a Precision) -> &'a Precision {
    fn rank(precision: &Precision) -> u8 {
        match precision {
            Precision::Analytic => 2,
            Precision::Approximate { .. } => 1,
            Precision::Unknown => 0,
        }
    }
    match (left, right) {
        (
            Precision::Approximate {
                error_bound: left_bound,
            },
            Precision::Approximate {
                error_bound: right_bound,
            },
        ) => {
            // Both approximate: the larger bound is the weaker claim, and an
            // unknown bound (`None`) is weaker than any stated one.
            match (left_bound, right_bound) {
                (None, _) => left,
                (_, None) => right,
                (Some(l), Some(r)) if l >= r => left,
                _ => right,
            }
        }
        _ if rank(left) <= rank(right) => left,
        _ => right,
    }
}
