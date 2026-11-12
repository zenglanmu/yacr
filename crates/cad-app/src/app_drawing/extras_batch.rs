//! The 2026-11-07 extras batch write paths: `SKETCH`, `TRACE`, `MOCORO` and
//! `LINETYPE`.
//!
//! Every write validates its whole input **before** opening a transaction and
//! then commits one transaction / one undo step. `SKETCH`/`TRACE` create one
//! LWPOLYLINE on the session's active layer (no `SKETCH`/`TRACE` entity type
//! exists — see [`crate::command_id::extras`]); `MOCORO` reuses the existing
//! MOVE/COPY/ROTATE/SCALE write paths; and `LINETYPE SET` reuses
//! `SETENTITYPROPERTY` (the one entity-property write path). `LINETYPE LIST` is
//! read-only.

use super::*;

use crate::{LinetypeCommand, MocoroCommand};

fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// Re-issue the same command envelope with another id/payload.
fn rerouted(command: &Command, id: CommandId, payload: CommandPayload) -> Command {
    Command {
        schema_version: command.schema_version,
        id,
        document: command.document,
        viewport: command.viewport,
        payload,
    }
}

impl Application {
    /// `SKETCH`: one open polyline from the already-sampled point sequence.
    ///
    /// The host owns freehand sampling; there is no timer, pointer capture or
    /// auto-close here. Fewer than two points and any non-finite point are
    /// refused before the transaction opens.
    pub(crate) fn sketch(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(refusal(
                "sketch.payload",
                "SKETCH needs a Points(<2 or more world points>) payload from the host sampler",
            ));
        };
        if points.len() < 2 {
            return Err(refusal(
                "sketch.too_few_points",
                format!("SKETCH needs at least 2 points, got {}", points.len()),
            ));
        }
        if let Some(point) = points.iter().find(|point| !is_finite_point(**point)) {
            return Err(refusal(
                "sketch.invalid_point",
                format!("SKETCH points must be finite, got {point:?}"),
            ));
        }
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Polyline {
                points: points.clone(),
                bulges: vec![0.0; points.len()],
                closed: false,
                width: 0.0,
            },
            "create sketch polyline",
        )
    }

    /// `TRACE`: one closed 4-vertex band from `start` to `end` and `width`.
    ///
    /// Stored as a closed `Polyline` in the XY plane (all vertices take the start
    /// point's Z). No `TRACE` entity type exists. A non-finite point, coincident
    /// endpoints and a non-finite/non-positive width are refused before the
    /// transaction opens.
    pub(crate) fn trace(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Trace { start, end, width } = &command.payload else {
            return Err(refusal(
                "trace.payload",
                "TRACE needs a Trace { start, end, width } payload",
            ));
        };
        if !is_finite_point(*start) || !is_finite_point(*end) {
            return Err(refusal(
                "trace.invalid_point",
                "TRACE start and end points must be finite",
            ));
        }
        if !width.is_finite() || *width <= 0.0 {
            return Err(refusal(
                "trace.invalid_width",
                format!("TRACE width must be finite and > 0, got {width}"),
            ));
        }
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let length = (dx * dx + dy * dy).sqrt();
        if !length.is_finite() || length <= 1e-12 {
            return Err(refusal(
                "trace.degenerate_direction",
                "TRACE needs two distinct points",
            ));
        }
        let half = width * 0.5;
        let nx = -dy / length * half;
        let ny = dx / length * half;
        let z = start.z;
        let points = vec![
            Point3 {
                x: start.x + nx,
                y: start.y + ny,
                z,
            },
            Point3 {
                x: end.x + nx,
                y: end.y + ny,
                z,
            },
            Point3 {
                x: end.x - nx,
                y: end.y - ny,
                z,
            },
            Point3 {
                x: start.x - nx,
                y: start.y - ny,
                z,
            },
        ];
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Polyline {
                bulges: vec![0.0; points.len()],
                points,
                closed: true,
                width: 0.0,
            },
            "create trace band",
        )
    }

    /// `MOCORO`: one typed transform over the current selection.
    ///
    /// The selection is read at apply time. `COPY` inserts translated clones;
    /// `MOVE`/`ROTATE`/`SCALE` transform in place. Every mode delegates to the
    /// existing command, so its supported-kind rules and its single
    /// transaction/undo step are exactly the ordinary command's. The
    /// **base point** is carried by the payload (`ROTATE`/`SCALE` take `base`;
    /// `MOVE`/`COPY` take the base-relative displacement), and each accepted
    /// invocation is one undo step — the AutoCAD in-command `Undo` is the global
    /// `UNDO` step, and there is no interactive option loop/gizmo in this build.
    ///
    /// Every number is validated here **before** the delegated command opens a
    /// transaction, so a bad value is a stable `mocoro.*` refusal that writes
    /// nothing and records no history (never an underlying generic message).
    pub(crate) fn mocoro(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Mocoro(operation) = &command.payload else {
            return Err(refusal(
                "mocoro.payload",
                "MOCORO needs a Mocoro(operation) payload",
            ));
        };
        validate_mocoro(*operation)?;
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(refusal(
                "mocoro.selection_required",
                "MOCORO needs at least one selected entity",
            ));
        }
        let mut outcome = match operation {
            MocoroCommand::Move { delta } => {
                let id = CommandId::MoveEntities;
                self.move_entities(&rerouted(
                    command,
                    id,
                    CommandPayload::Move {
                        refs,
                        delta: *delta,
                    },
                ))
            }
            MocoroCommand::Copy { delta } => {
                let id = CommandId::CopyEntities;
                self.copy_entities(&rerouted(
                    command,
                    id,
                    CommandPayload::Copy {
                        refs,
                        delta: *delta,
                    },
                ))
            }
            MocoroCommand::Rotate { base, degrees } => {
                let id = CommandId::RotateEntities;
                self.rotate_entities(&rerouted(
                    command,
                    id,
                    CommandPayload::Rotate {
                        refs,
                        base: *base,
                        angle: degrees.to_radians(),
                    },
                ))
            }
            MocoroCommand::Scale { base, factor } => {
                let id = CommandId::ScaleEntities;
                self.scale_entities(&rerouted(
                    command,
                    id,
                    CommandPayload::Scale {
                        refs,
                        base: *base,
                        factor: *factor,
                    },
                ))
            }
        }?;
        outcome.diagnostics.insert(
            0,
            Diagnostic {
                object: None,
                code: "drawing.mocoro".into(),
                message: format!(
                    "MOCORO {}: one transaction / one undo step over the current selection",
                    mocoro_label(*operation)
                ),
            },
        );
        Ok(outcome)
    }

    /// `LINETYPE LIST` / `LINETYPE SET <name>`.
    ///
    /// `LIST` reads the drawing's linetype table and writes nothing. `SET`
    /// re-authorizes the ordinary work-only entity-property write, then routes
    /// the name through [`Self::set_entity_property`] — the one property grammar
    /// and the one transaction/undo path. There is no linetype-table write/load
    /// API in this build.
    pub(crate) fn linetype(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Linetype(option) = &command.payload else {
            return Err(refusal(
                "linetype.payload",
                "LINETYPE needs LIST or SET <name>",
            ));
        };
        match option {
            LinetypeCommand::List => {
                let drawing = self
                    .workspace
                    .documents
                    .get(&command.document)
                    .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
                let names: Vec<&str> = drawing
                    .drawing
                    .linetypes()
                    .map(|lt| lt.name.as_str())
                    .collect();
                let message = if names.is_empty() {
                    "LINETYPE LIST: the drawing has no linetype table entries".to_string()
                } else {
                    format!(
                        "LINETYPE LIST: {} linetype(s): {}",
                        names.len(),
                        names.join(", ")
                    )
                };
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "linetype.list".into(),
                        message,
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            LinetypeCommand::Set(name) => {
                // `LINETYPE` is admitted in Viewer for LIST; the SET half writes
                // entity rows, so it goes through the ordinary work-only
                // authorization rather than a second mode rule.
                session.authorize(CommandId::SetEntityProperty)?;
                let id = CommandId::SetEntityProperty;
                self.set_entity_property(
                    session,
                    &rerouted(
                        command,
                        id,
                        CommandPayload::EntityProperty {
                            key: "linetype".into(),
                            value: name.clone(),
                        },
                    ),
                )
            }
        }
    }
}

/// The stable mode name MOCORO reports in its attribution diagnostic.
fn mocoro_label(operation: MocoroCommand) -> &'static str {
    match operation {
        MocoroCommand::Move { .. } => "MOVE",
        MocoroCommand::Copy { .. } => "COPY",
        MocoroCommand::Rotate { .. } => "ROTATE",
        MocoroCommand::Scale { .. } => "SCALE",
    }
}

/// Validate every MOCORO number before the delegated command can write.
fn validate_mocoro(operation: MocoroCommand) -> CadResult<()> {
    match operation {
        MocoroCommand::Move { delta } | MocoroCommand::Copy { delta } => {
            if !is_finite_point(delta) {
                return Err(refusal(
                    "mocoro.invalid_delta",
                    format!("the MOCORO displacement must be finite, got {delta:?}"),
                ));
            }
        }
        MocoroCommand::Rotate { base, degrees } => {
            if !is_finite_point(base) {
                return Err(refusal(
                    "mocoro.invalid_base",
                    format!("the MOCORO base point must be finite, got {base:?}"),
                ));
            }
            if !degrees.is_finite() {
                return Err(refusal(
                    "mocoro.invalid_angle",
                    format!("the MOCORO rotation must be a finite angle, got {degrees}"),
                ));
            }
        }
        MocoroCommand::Scale { base, factor } => {
            if !is_finite_point(base) {
                return Err(refusal(
                    "mocoro.invalid_base",
                    format!("the MOCORO base point must be finite, got {base:?}"),
                ));
            }
            if !factor.is_finite() || factor <= 0.0 {
                return Err(refusal(
                    "mocoro.invalid_factor",
                    format!("the MOCORO scale factor must be finite and positive, got {factor}"),
                ));
            }
        }
    }
    Ok(())
}
