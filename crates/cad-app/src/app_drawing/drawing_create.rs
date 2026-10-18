use super::*;

impl Application {
    /// Dispatch the four drawing/edit commands.
    pub(crate) fn drawing_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::CreateLine => self.create_line(session, command),
            CommandId::CreateCircle => self.create_circle(session, command),
            CommandId::CreateArc => self.create_arc(session, command),
            CommandId::MoveEntities => self.move_entities(command),
            CommandId::TrimEntity => self.trim_entity(command),
            CommandId::ExtendEntity => self.extend_entity(command),
            CommandId::EraseEntities => self.erase_entities(session, command),
            CommandId::CopyEntities => self.copy_entities(command),
            CommandId::RotateEntities => self.rotate_entities(command),
            CommandId::ScaleEntities => self.scale_entities(command),
            CommandId::MirrorEntities => self.mirror_entities(command),
            CommandId::CopyToClipboard => self.copy_to_clipboard(session, command),
            CommandId::CutToClipboard => self.cut_to_clipboard(session, command),
            CommandId::PasteFromClipboard => self.paste_from_clipboard(session, command),
            CommandId::OffsetEntity => self.offset_entity(command),
            CommandId::SetActiveLayer => self.set_active_layer(session, command),
            _ => Err(CadError::InvalidInput(
                "not a drawing/edit command".to_string(),
            )),
        }
    }

    /// Set the session's active drawing layer (session state only).
    ///
    /// The layer need not exist yet: the *create* commands are the point where a
    /// missing layer becomes an explicit `InvalidInput`, so a UI may set a
    /// preference before the table is fully known. This command performs no
    /// database write and records no history.
    pub(crate) fn set_active_layer(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ActiveLayer(layer) = command.payload else {
            return Err(CadError::InvalidInput(
                "SetActiveLayer needs an ActiveLayer(LayerId) payload".into(),
            ));
        };
        session.active_layer = layer;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.active_layer".into(),
                message: format!("活动图层：{}", layer.0),
            }],
            measurement: None,
        })
    }

    /// Insert a LINE into model space on the session's active layer.
    ///
    /// Accepts either `Points([start, end])` (the UI path) or a fully formed
    /// `Geometry(SemanticGeometry::Line { .. })` (the programmatic path). The
    /// geometry is validated by the database before it is committed, so a
    /// non-finite or degenerate line is refused with no state change.
    pub(crate) fn create_line(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(points) => {
                let start = *points.first().ok_or_else(|| {
                    CadError::InvalidInput("CreateLine needs a start point".into())
                })?;
                let end = *points.get(1).ok_or_else(|| {
                    CadError::InvalidInput("CreateLine needs an end point".into())
                })?;
                SemanticGeometry::Line { start, end }
            }
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Line { start, end } => SemanticGeometry::Line {
                    start: *start,
                    end: *end,
                },
                other => {
                    return Err(CadError::InvalidInput(format!(
                        "CreateLine needs a Line geometry, got {}",
                        geometry_kind(other)
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "CreateLine needs a Points([start, end]) or Geometry payload".into(),
                ));
            }
        };
        self.commit_new_entity(session, command, geometry, "create line")
    }

    /// Insert a CIRCLE with radius = |edge - center| into model space.
    ///
    /// A zero (or non-finite) radius is an explicit `InvalidInput` and leaves
    /// the database revision unchanged, never a degenerate circle.
    pub(crate) fn create_circle(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateCircle needs a Points([center, edge]) payload".into(),
            ));
        };
        let center = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("CreateCircle needs a center point".into()))?;
        let edge = *points
            .get(1)
            .ok_or_else(|| CadError::InvalidInput("CreateCircle needs an edge point".into()))?;
        let radius = distance(center, edge);
        if !radius.is_finite() || radius <= 0.0 {
            return Err(CadError::InvalidInput(
                "CreateCircle radius must be a positive finite distance".into(),
            ));
        }
        let geometry = SemanticGeometry::Circle {
            center,
            // The zero-radius guard above, plus the finite-radius check, means
            // the normal is only a documented plane hint; a flat circle uses the
            // work-plane +Z normal, never a fabricated tilt.
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        };
        self.commit_new_entity(session, command, geometry, "create circle")
    }

    /// Insert an ARC through three points into model space.
    ///
    /// Accepts `Points([start, through, end])` (the UI path) or a fully formed
    /// `Geometry(SemanticGeometry::Arc { .. })` (the programmatic path). The
    /// circumcircle is a work-plane `+Z` construction; collinear, coincident or
    /// non-finite points are an explicit refusal that creates nothing.
    pub(crate) fn create_arc(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(points) => {
                let start = *points.first().ok_or_else(|| {
                    CadError::InvalidInput("CreateArc needs a start point".into())
                })?;
                let through = *points.get(1).ok_or_else(|| {
                    CadError::InvalidInput("CreateArc needs an on-arc point".into())
                })?;
                let end = *points
                    .get(2)
                    .ok_or_else(|| CadError::InvalidInput("CreateArc needs an end point".into()))?;
                let parameters = crate::draw_tool::arc_parameters(start, through, end)?;
                SemanticGeometry::Arc {
                    center: parameters.center,
                    // The circumcircle is a work-plane construction; a flat arc
                    // uses +Z, never a fabricated tilt.
                    normal: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 1.0,
                    },
                    radius: parameters.radius,
                    start: parameters.start,
                    sweep: parameters.sweep,
                }
            }
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Arc {
                    center,
                    normal,
                    radius,
                    start,
                    sweep,
                } => SemanticGeometry::Arc {
                    center: *center,
                    normal: *normal,
                    radius: *radius,
                    start: *start,
                    sweep: *sweep,
                },
                other => {
                    return Err(CadError::InvalidInput(format!(
                        "CreateArc needs an Arc geometry, got {}",
                        geometry_kind(other)
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "CreateArc needs a Points([start, through, end]) or Geometry payload".into(),
                ));
            }
        };
        self.commit_new_entity(session, command, geometry, "create arc")
    }
}
