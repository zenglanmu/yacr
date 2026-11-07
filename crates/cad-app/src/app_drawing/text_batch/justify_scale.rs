//! `JUSTIFYTEXT`/`TJUST` (alignment-only rewrite) and `TSCALE` (the shared
//! `SCALETEXT` scale rule driven by a typed factor).

use super::shared::*;
use super::*;

impl Application {
    /// `JUSTIFYTEXT <H> <V>` / `TJUST <H> <V>`: rewrite only the alignment pair.
    ///
    /// Non-text and non-addressable targets are per-entity `justifytext.not_text`
    /// diagnostics (the `TORIENT` policy); the command only fails when the
    /// selection is empty. A change-free result is `justifytext.unchanged`.
    pub(crate) fn justify_text(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::JustifyText(request) = &command.payload else {
            return Err(refusal(
                "justifytext.invalid_payload",
                "JUSTIFYTEXT needs a JustifyText { h_align, v_align } payload \
                 (type `JUSTIFYTEXT <H> <V>`)",
            ));
        };
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "justifytext.needs_selection",
                "JUSTIFYTEXT needs at least one selected Text/MText entity",
            ));
        }
        let document_id = command.document;
        let candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let mut diagnostics = Vec::new();
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for reference in &references {
            let before =
                match resolve_model_entity(&candidate, document_id, reference, "justifytext") {
                    Ok(entity) => entity,
                    Err(error) => {
                        diagnostics.push(skipped(
                            "justifytext.not_text",
                            reference.entity,
                            &error_detail(error),
                        ));
                        continue;
                    }
                };
            let SemanticGeometry::Text {
                h_align, v_align, ..
            } = &before.geometry
            else {
                diagnostics.push(skipped(
                    "justifytext.not_text",
                    before.id,
                    &format!("{} is not a Text/MText entity", kind_name(&before.geometry)),
                ));
                continue;
            };
            if *h_align == request.h_align && *v_align == request.v_align {
                continue;
            }
            let mut after = before.clone();
            if let SemanticGeometry::Text {
                h_align, v_align, ..
            } = &mut after.geometry
            {
                *h_align = request.h_align;
                *v_align = request.v_align;
            }
            planned.push((before, after));
        }
        let selected = references.len();
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                "justifytext.unchanged",
                format!(
                    "JUSTIFYTEXT: none of the {selected} selected entities needed the requested \
                     alignment; nothing was written"
                ),
                diagnostics,
            ));
        }
        let (objects, change_set) = commit_text_updates(
            self,
            document_id,
            candidate,
            &planned,
            "justify text",
            "justify text",
        )?;
        let mut all = diagnostics;
        all.push(Diagnostic {
            object: None,
            code: "drawing.justifytext".into(),
            message: format!(
                "JUSTIFYTEXT: set {:?}/{:?} on {} of {selected} selected text entities \
                 (position unchanged)",
                request.h_align,
                request.v_align,
                objects.len()
            ),
        });
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: all,
            measurement: None,
            point_report: None,
        })
    }

    /// `TSCALE <factor>`: scale the selected text with the shared `scaled_text`
    /// rule (`SCALETEXT`'s only scale implementation).
    pub(crate) fn text_scale(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::TextScale(request) = &command.payload else {
            return Err(refusal(
                "tscale.invalid_payload",
                "TSCALE needs a TextScale { factor } payload (type `TSCALE <factor>`)",
            ));
        };
        let factor = request.factor;
        if !factor.is_finite() || factor <= 0.0 {
            return Err(refusal(
                "tscale.invalid_factor",
                format!("TSCALE factor must be finite and > 0, got {factor}"),
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "tscale.needs_selection",
                "TSCALE needs at least one selected Text/MText entity",
            ));
        }
        let document_id = command.document;
        let candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let mut sources = Vec::with_capacity(references.len());
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            let entity = resolve_model_entity(&candidate, document_id, reference, "tscale")?;
            require_text("tscale", &entity)?;
            if seen.insert(entity.id) {
                sources.push(entity);
            }
        }
        if factor == 1.0 {
            return Ok(unchanged_outcome(
                "tscale.unchanged",
                format!(
                    "TSCALE: factor {factor} leaves all {} selected text entities unchanged",
                    sources.len()
                ),
                Vec::new(),
            ));
        }
        let mut planned = Vec::with_capacity(sources.len());
        for before in sources {
            let after = super::scale_text::scaled_text(&before, factor)?;
            if after.geometry != before.geometry {
                planned.push((before, after));
            }
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                "tscale.unchanged",
                "TSCALE: no selected text entity would change; nothing was written".into(),
                Vec::new(),
            ));
        }
        let (objects, change_set) = commit_text_updates(
            self,
            document_id,
            candidate,
            &planned,
            "text scale",
            "text scale",
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.tscale".into(),
                message: format!(
                    "TSCALE: scaled {} text entities by factor {factor} (SCALETEXT rule)",
                    planned.len()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
