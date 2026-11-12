//! `ADDSELECTED <x> <y>`: create a new entity of the single selected entity's
//! type and general properties at the typed point.
//!
//! The payload is [`crate::CommandPayload::AddSelected`] `{ position }`; the
//! selection is the source. Exactly one **directly addressable model-space**
//! entity must be selected, and its layer, stored `type_key` and per-entity
//! render attributes (colour, linetype, lineweight, transparency, annotative
//! flag) are copied onto the new row through the shared create funnel
//! ([`Application::commit_new_entity_with_display`]: one transaction, one undo
//! step, all-or-nothing).
//!
//! ## Supported types (point-anchored)
//!
//! | source | new geometry |
//! | --- | --- |
//! | `LINE` | `start = position`, `end = position + (end − start)` (the source's own direction and length) |
//! | `CIRCLE` | `center = position`, same normal and radius |
//! | `POINT` | the typed position |
//! | `TEXT` / `MTEXT` | insertion `position`; text, style, height, rotation, font, alignment and wrap width are all preserved |
//!
//! Every other kind needs richer input than one point (an ARC's sweep, a
//! POLYLINE's vertex list, an INSERT's block name, …), so it is the named
//! `addselected.unsupported` refusal rather than a silently wrong entity.
//!
//! Refusals: `addselected.payload`, `addselected.no_selection`,
//! `addselected.multiple`, `addselected.not_addressable` (another document, a
//! block-instance member, a sub-element or a missing row),
//! `addselected.not_model_space`, `addselected.invalid_point` and
//! `addselected.unsupported`.

use super::*;

/// An `addselected.<code>` refusal.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// Create one entity of the selected entity's type at `position`.
    pub(crate) fn add_selected(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::AddSelected(payload) = &command.payload else {
            return Err(refusal(
                "addselected.payload",
                "ADDSELECTED needs an AddSelected payload (x, y); the typed form is \
                 `ADDSELECTED <x> <y>`",
            ));
        };
        let position = payload.position;
        if !is_finite_point(position) {
            return Err(refusal(
                "addselected.invalid_point",
                format!("the ADDSELECTED placement point {position:?} is not finite"),
            ));
        }
        let references = session.selection.refs();
        let reference = match references {
            [] => {
                return Err(refusal(
                    "addselected.no_selection",
                    "ADDSELECTED needs exactly one selected entity; nothing is selected",
                ));
            }
            [one] => one,
            many => {
                return Err(refusal(
                    "addselected.multiple",
                    format!(
                        "ADDSELECTED reads exactly one source entity; {} are selected",
                        many.len()
                    ),
                ));
            }
        };
        let document_id = command.document;
        let (source, display) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            if reference.document != document_id
                || !reference.instance.0.is_empty()
                || reference.sub_element.is_some()
            {
                return Err(refusal(
                    "addselected.not_addressable",
                    format!(
                        "entity {} is not a directly addressable entity in this document",
                        reference.entity.0
                    ),
                ));
            }
            let source = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                refusal(
                    "addselected.not_addressable",
                    format!(
                        "entity {} does not exist in this drawing",
                        reference.entity.0
                    ),
                )
            })?;
            if source.space != SpaceId::Model {
                return Err(refusal(
                    "addselected.not_model_space",
                    format!(
                        "entity {} is not in model space; ADDSELECTED creates model-space rows only",
                        source.id.0
                    ),
                ));
            }
            let display = drawing.entity_display_state(source.id);
            (source, display)
        };

        let geometry = match &source.geometry {
            SemanticGeometry::Line { start, end } => {
                let offset = Point3 {
                    x: end.x - start.x,
                    y: end.y - start.y,
                    z: end.z - start.z,
                };
                SemanticGeometry::Line {
                    start: position,
                    end: Point3 {
                        x: position.x + offset.x,
                        y: position.y + offset.y,
                        z: position.z + offset.z,
                    },
                }
            }
            SemanticGeometry::Circle { normal, radius, .. } => SemanticGeometry::Circle {
                center: position,
                normal: *normal,
                radius: *radius,
            },
            SemanticGeometry::Point(_) => SemanticGeometry::Point(position),
            SemanticGeometry::Text {
                text,
                style,
                height,
                rotation,
                font,
                h_align,
                v_align,
                wrap_width,
                width_factor,
                ..
            } => SemanticGeometry::Text {
                text: text.clone(),
                position,
                style: *style,
                height: *height,
                rotation: *rotation,
                font: font.clone(),
                h_align: *h_align,
                v_align: *v_align,
                wrap_width: *wrap_width,
                width_factor: *width_factor,
            },
            other => {
                return Err(unsupported(
                    "addselected.unsupported",
                    format!(
                        "ADDSELECTED supports point-anchored LINE/CIRCLE/POINT/TEXT; {} needs \
                         richer input than one point",
                        geometry_kind(other)
                    ),
                ));
            }
        };

        let mut outcome = self.commit_new_entity_with_display(
            command,
            geometry,
            source.layer,
            SpaceId::Model,
            Some(display),
            Some(source.object.type_key.as_str()),
            "add selected",
        )?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "drawing.addselected".into(),
            message: format!(
                "ADDSELECTED: created one {} from entity {} at ({}, {}, {}) on layer {} with its \
                 colour/linetype/lineweight/transparency",
                class_name(&source.geometry),
                source.id.0,
                position.x,
                position.y,
                position.z,
                source.layer.0
            ),
        });
        Ok(outcome)
    }
}

/// An `addselected.<code>` `Unsupported` refusal.
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}
