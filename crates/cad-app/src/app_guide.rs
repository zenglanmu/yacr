//! Persisted construction guides (`tables/guides.json`, minor 26).
//!
//! Guides are non-entity infinite lines created by `DIST GUIDE` /
//! `PROTRACTOR GUIDE`. The table is bounded and written whole; `GUIDE LIST` is a
//! read-only report and `GUIDE CLEAR` is one revision/undo step. Guides never
//! enter the entity store, exports or picking.

use super::*;
use cad_domain::guide::{Guide, GuideLine, GuidePoint};

impl Application {
    /// Write the guide a guide-mode measurement asked for.
    pub(crate) fn add_measurement_guide(
        &mut self,
        document: DocumentId,
        mode: crate::GuideMode,
        points: &[Point3],
    ) -> CadResult<()> {
        let guide = match mode {
            crate::GuideMode::DistanceLine => {
                points.first().zip(points.get(1)).and_then(|(start, end)| {
                    // SketchUp's Tape Measure leaves a point guide when the two
                    // picks are the same point; within this tolerance the line
                    // direction is not meaningful, so mark the point instead of
                    // refusing the measurement.
                    let distance = {
                        let dx = end.x - start.x;
                        let dy = end.y - start.y;
                        let dz = end.z - start.z;
                        dx.hypot(dy).hypot(dz)
                    };
                    if distance <= 1e-9 {
                        GuidePoint::new(*start).map(Guide::Point)
                    } else {
                        GuideLine::through(*start, *end).map(Guide::Line)
                    }
                })
            }
            crate::GuideMode::ProtractorLine => {
                let vertex = *points.first().ok_or_else(|| {
                    CadError::InvalidInput("guide.degenerate: the protractor has no vertex".into())
                })?;
                let ray = *points.get(2).ok_or_else(|| {
                    CadError::InvalidInput(
                        "guide.degenerate: the protractor has no second ray".into(),
                    )
                })?;
                GuideLine::new(
                    vertex,
                    Point3 {
                        x: ray.x - vertex.x,
                        y: ray.y - vertex.y,
                        z: ray.z - vertex.z,
                    },
                )
                .map(Guide::Line)
            }
        }
        .ok_or_else(|| {
            CadError::InvalidInput(
                "guide.degenerate: the measured points do not define a guide direction".into(),
            )
        })?;
        let mut next = self.drawing(document)?.guides().clone();
        next.push(guide)
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        self.write_guides(document, next, "GUIDE (measurement)")
    }

    /// `GUIDE [LIST|CLEAR]`.
    pub(crate) fn guide_command(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let option = match &command.payload {
            CommandPayload::Guide(option) => *option,
            CommandPayload::None => GuideCommand::List,
            _ => return Err(CadError::InvalidInput("GUIDE takes LIST or CLEAR".into())),
        };
        match option {
            GuideCommand::List => {
                let table = self.drawing(command.document)?.guides();
                let message = if table.is_empty() {
                    "GUIDE LIST: no construction guides in this document".to_string()
                } else {
                    let rows = table
                        .guides()
                        .iter()
                        .take(8)
                        .enumerate()
                        .map(|(index, guide)| match guide {
                            Guide::Line(line) => format!(
                                "{}. line ({:.4}, {:.4}, {:.4}) dir ({:.4}, {:.4}, {:.4})",
                                index + 1,
                                line.point.x,
                                line.point.y,
                                line.point.z,
                                line.direction.x,
                                line.direction.y,
                                line.direction.z
                            ),
                            Guide::Point(point) => format!(
                                "{}. point ({:.4}, {:.4}, {:.4})",
                                index + 1,
                                point.point.x,
                                point.point.y,
                                point.point.z
                            ),
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    format!(
                        "GUIDE LIST: {} guide(s): {rows}{}",
                        table.len(),
                        if table.len() > 8 { " …" } else { "" }
                    )
                };
                Ok(guide_outcome("guide.list", message))
            }
            GuideCommand::Clear => {
                if session.app_mode() == AppMode::Viewer {
                    return Err(CadError::PermissionDenied);
                }
                if self.drawing(command.document)?.guides().is_empty() {
                    return Ok(guide_outcome(
                        "guide.no_change",
                        "GUIDE CLEAR: the document has no guides; nothing was written".into(),
                    ));
                }
                self.write_guides(
                    command.document,
                    cad_domain::guide::GuideTable::new(),
                    "GUIDE CLEAR",
                )?;
                Ok(guide_outcome(
                    "guide.cleared",
                    "GUIDE CLEAR: removed every construction guide".into(),
                ))
            }
            GuideCommand::Delete(index) => {
                if session.app_mode() == AppMode::Viewer {
                    return Err(CadError::PermissionDenied);
                }
                let mut next = self.drawing(command.document)?.guides().clone();
                if next.remove(index.saturating_sub(1)).is_none() {
                    return Err(CadError::InvalidInput(format!(
                        "guide.unknown_index: there is no guide at row {index} (the document has \
                         {} guide(s))",
                        next.len()
                    )));
                }
                self.write_guides(command.document, next, &format!("GUIDE DELETE {index}"))?;
                Ok(guide_outcome(
                    "guide.deleted",
                    format!("GUIDE DELETE: removed guide {index}"),
                ))
            }
            GuideCommand::Point(point) => {
                if session.app_mode() == AppMode::Viewer {
                    return Err(CadError::PermissionDenied);
                }
                let guide = GuidePoint::new(point).ok_or_else(|| {
                    CadError::InvalidInput(
                        "guide.invalid: a guide point needs finite coordinates".into(),
                    )
                })?;
                let mut next = self.drawing(command.document)?.guides().clone();
                next.push_point(guide)
                    .map_err(|error| CadError::InvalidInput(error.to_string()))?;
                self.write_guides(command.document, next, "GUIDE POINT")?;
                Ok(guide_outcome(
                    "guide.point",
                    format!(
                        "GUIDE POINT: marked ({:.4}, {:.4}, {:.4})",
                        point.x, point.y, point.z
                    ),
                ))
            }
        }
    }

    fn write_guides(
        &mut self,
        document: DocumentId,
        next: cad_domain::guide::GuideTable,
        label: &str,
    ) -> CadResult<()> {
        let before = self.drawing(document)?.guides().clone();
        let transaction = crate::app_drawing::next_transaction_id();
        {
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            std::sync::Arc::make_mut(&mut document.drawing).apply_guides(
                next.clone(),
                transaction,
                label,
            )?;
        }
        let patch = cad_db::GuidePatch {
            before: before.clone(),
            after: next,
        };
        let record =
            cad_history::DrawingUndoRecord::entity_only(transaction, label.to_string(), Vec::new());
        if let Err(error) = self
            .history
            .entry(document)
            .or_default()
            .record_drawing_with_guides(record, vec![patch])
        {
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            std::sync::Arc::make_mut(&mut document.drawing).apply_guides(
                before,
                crate::app_drawing::next_transaction_id(),
                "GUIDE rollback",
            )?;
            return Err(error);
        }
        Ok(())
    }
}

fn guide_outcome(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}
