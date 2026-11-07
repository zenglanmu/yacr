//! `PEDIT` v1 — the declared polyline-option subset `CLOSE`, `OPEN`, `REVERSE`
//! (AutoCAD 2026).
//!
//! AutoCAD's `PEDIT` picks one polyline and then shows an option prompt. This
//! build has **no in-command option prompt**, so the option is typed as part of
//! the command line (`PEDIT CLOSE|OPEN|REVERSE`) and carried in
//! [`CommandPayload::PeditOption`]. The current selection is the whole input:
//! **exactly one** directly addressable model-space LWPOLYLINE. There is no
//! interactive "Select polyline" prompt and no `Multiple` mode.
//!
//! Every refusal is named and happens **before** a transaction opens; the three
//! real writes stage one `update_entity` in one transaction / one undo step. The
//! entity keeps its id, type key, layer, space and draw order — only
//! `points`/`bulges`/`closed` change. Full contract:
//! [`crate::command_id::pedit`].

use super::*;

use crate::PeditOption;

/// A `pedit.<code>: <detail>` refusal string (ASCII only).
fn refusal(code: &str, detail: impl std::fmt::Display) -> String {
    format!("pedit.{code}: {detail}")
}

/// A named, non-capability refusal (bad input for a supported shape).
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(refusal(code, detail))
}

/// A named capability-gap refusal (nothing is written).
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(refusal(code, detail))
}

/// The named no-op outcome (`pedit.already_closed` / `pedit.already_open`):
/// no transaction, no revision bump, no undo entry.
fn unchanged(code: &str, message: String) -> CommandOutcome {
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

impl Application {
    /// `PEDIT`: apply one declared v1 polyline option to the one selected
    /// LWPOLYLINE (one transaction / one undo step for a real change).
    pub(crate) fn pedit(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let option = match command.payload {
            CommandPayload::PeditOption(option) => option,
            CommandPayload::None => {
                return Err(invalid(
                    "needs_option",
                    "PEDIT needs an option: CLOSE, OPEN or REVERSE (e.g. `PEDIT CLOSE`). \
                     Join, Width, Halfwidth, Fit, Spline, Decurve, Ltype gen, Undo, Multiple, \
                     Edit vertex and the PEDITACCEPT conversion prompt are NOT implemented",
                ));
            }
            _ => {
                return Err(invalid(
                    "payload",
                    "PEDIT takes a PeditOption payload; the selection is the input",
                ));
            }
        };

        let references = session.selection.refs();
        if references.len() != 1 {
            return Err(invalid(
                "selection",
                format!(
                    "PEDIT edits exactly one polyline, got {} selected (the interactive \
                     Select polyline / Multiple prompt is not implemented)",
                    references.len()
                ),
            ));
        }
        let reference = &references[0];
        if reference.document != command.document {
            return Err(invalid(
                "not_addressable",
                format!(
                    "the selected ref for entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "PEDIT needs a directly addressable model-space polyline; entity {} is \
                     referenced through a block instance or a sub-element",
                    reference.entity.0
                ),
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                "not_addressable",
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        if entity.space != SpaceId::Model {
            return Err(unsupported(
                "not_addressable",
                format!(
                    "PEDIT v1 edits model-space polylines only; entity {} is in another space",
                    reference.entity.0
                ),
            ));
        }
        let (points, bulges, closed) = match &entity.geometry {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => (points.clone(), bulges.clone(), *closed),
            other => {
                return Err(unsupported(
                    "not_polyline",
                    format!(
                        "PEDIT needs a POLYLINE selection; {} is not supported (the \
                         PEDITACCEPT line/arc conversion prompt is not implemented)",
                        class_name(other)
                    ),
                ));
            }
        };

        // A no-op reports its named state and writes nothing: no transaction, no
        // revision bump and no undo entry.
        let after_geometry = match option {
            PeditOption::Close => {
                if closed {
                    return Ok(unchanged(
                        "pedit.already_closed",
                        format!(
                            "PEDIT CLOSE: entity {} is already closed; nothing was written",
                            entity.id.0
                        ),
                    ));
                }
                if points.len() < 3 {
                    return Err(invalid(
                        "too_few_vertices",
                        format!(
                            "PEDIT CLOSE needs at least three vertices to form a closed ring, \
                             got {}",
                            points.len()
                        ),
                    ));
                }
                SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed: true,
                }
            }
            PeditOption::Open => {
                if !closed {
                    return Ok(unchanged(
                        "pedit.already_open",
                        format!(
                            "PEDIT OPEN: entity {} is already open; nothing was written",
                            entity.id.0
                        ),
                    ));
                }
                SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed: false,
                }
            }
            PeditOption::Reverse => {
                // The **same** pure rule as REVERSE (bulges reversed and negated
                // by the one shared mapping); never a second implementation.
                super::modify_extra::reverse_geometry(&entity.geometry).map_err(
                    |error| match error {
                        CadError::InvalidInput(message) => {
                            CadError::InvalidInput(refusal("reverse", message))
                        }
                        other => other,
                    },
                )?
            }
        };

        let entity_id = entity.id;
        let mut after = entity.clone();
        after.geometry = after_geometry;
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("pedit", transaction)?;
            staged.update_entity(after)?;
            staged.commit()?
        };
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let stored = drawing.entity(entity_id).cloned().ok_or_else(|| {
            CadError::Invariant("PEDIT: the polyline vanished after commit".into())
        })?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "pedit".to_string(),
                patches: vec![drawing_patch(entity_id, Some(entity), Some(stored))],
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        let label = match option {
            PeditOption::Close => "CLOSE (closed = true)",
            PeditOption::Open => "OPEN (closed = false)",
            PeditOption::Reverse => "REVERSE (vertex order and bulges reversed)",
        };
        Ok(CommandOutcome {
            objects: vec![ObjectId(entity_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity_id.0)),
                code: "pedit.applied".into(),
                // Locale-neutral technical diagnostic: English ASCII only.
                message: format!(
                    "PEDIT {label}: updated polyline entity {} in one transaction",
                    entity_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
