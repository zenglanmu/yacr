//! `TEXTALIGN` (common edge/centre from estimated extents) and `TCOUNT`
//! (deterministic reading-order numbering).

use super::shared::*;
use super::*;

impl Application {
    /// `TEXTALIGN <edge>`: move the selected text to one common edge/centre.
    pub(crate) fn text_align(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::TextAlignEdges(request) = &command.payload else {
            return Err(refusal(
                "textalign.invalid_payload",
                "TEXTALIGN needs a TextAlignEdges { edge } payload \
                 (type `TEXTALIGN LEFT|RIGHT|CENTER|TOP|MIDDLE|BOTTOM`)",
            ));
        };
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "textalign.needs_selection",
                "TEXTALIGN needs at least two selected Text/MText entities",
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
            let entity = resolve_model_entity(&candidate, document_id, reference, "textalign")?;
            require_text("textalign", &entity)?;
            if seen.insert(entity.id) {
                sources.push(entity);
            }
        }
        if sources.len() < 2 {
            return Err(refusal(
                "textalign.needs_two",
                format!(
                    "TEXTALIGN needs at least two Text/MText targets, got {}",
                    sources.len()
                ),
            ));
        }
        // One estimate per source, validated before any transaction.
        let mut bounds = Vec::with_capacity(sources.len());
        for source in &sources {
            bounds.push(estimate_text_bounds(source, "textalign")?);
        }
        let min_x = bounds.iter().map(|b| b.min_x).fold(f64::INFINITY, f64::min);
        let max_x = bounds
            .iter()
            .map(|b| b.max_x)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = bounds.iter().map(|b| b.min_y).fold(f64::INFINITY, f64::min);
        let max_y = bounds
            .iter()
            .map(|b| b.max_y)
            .fold(f64::NEG_INFINITY, f64::max);
        let common_x = match request.edge {
            TextAlignEdge::Left => Some(min_x),
            TextAlignEdge::Right => Some(max_x),
            TextAlignEdge::Center => Some((min_x + max_x) * 0.5),
            _ => None,
        };
        let common_y = match request.edge {
            TextAlignEdge::Top => Some(max_y),
            TextAlignEdge::Bottom => Some(min_y),
            TextAlignEdge::Middle => Some((min_y + max_y) * 0.5),
            _ => None,
        };
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for (source, box_) in sources.into_iter().zip(bounds) {
            let mut after = source.clone();
            let moved = match request.edge {
                TextAlignEdge::Left => shift_text(&mut after, box_.min_x, common_x.unwrap(), 0.0),
                TextAlignEdge::Right => shift_text(&mut after, box_.max_x, common_x.unwrap(), 0.0),
                TextAlignEdge::Center => {
                    let (cx, _) = box_.center();
                    shift_text(&mut after, cx, common_x.unwrap(), 0.0)
                }
                TextAlignEdge::Top => shift_text(&mut after, box_.max_y, common_y.unwrap(), 1.0),
                TextAlignEdge::Bottom => shift_text(&mut after, box_.min_y, common_y.unwrap(), 1.0),
                TextAlignEdge::Middle => {
                    let (_, cy) = box_.center();
                    shift_text(&mut after, cy, common_y.unwrap(), 1.0)
                }
            };
            if moved {
                planned.push((source, after));
            }
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                "textalign.unchanged",
                format!(
                    "TEXTALIGN {:?}: every target already lies on the common line; nothing was \
                     written",
                    request.edge
                ),
                Vec::new(),
            ));
        }
        let (objects, change_set) = commit_text_updates(
            self,
            document_id,
            candidate,
            &planned,
            "text align",
            "text align",
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.textalign".into(),
                message: format!(
                    "TEXTALIGN {:?}: moved {} of the selected text entities using the \
                     estimated bounds (width = height × character count, height = glyph \
                     height; rotation ignored)",
                    request.edge,
                    planned.len()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `TCOUNT <start> <step> [prefix] [suffix]`: number the selected text in
    /// reading order.
    pub(crate) fn text_count(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::TextCount(request) = &command.payload else {
            return Err(refusal(
                "tcount.invalid_payload",
                "TCOUNT needs a TextCount { start, step, prefix, suffix } payload",
            ));
        };
        if request.step <= 0 {
            return Err(refusal(
                "tcount.invalid_step",
                format!(
                    "TCOUNT step must be strictly positive, got {}",
                    request.step
                ),
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "tcount.needs_selection",
                "TCOUNT needs at least one selected Text/MText entity",
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
            let entity = resolve_model_entity(&candidate, document_id, reference, "tcount")?;
            require_text("tcount", &entity)?;
            if seen.insert(entity.id) {
                sources.push(entity);
            }
        }
        // Deterministic reading order: top-to-bottom (descending Y) then
        // left-to-right (ascending X), stable — the TXT2MTXT sort.
        sources.sort_by(|first, second| {
            let (
                SemanticGeometry::Text { position: a, .. },
                SemanticGeometry::Text { position: b, .. },
            ) = (&first.geometry, &second.geometry)
            else {
                return std::cmp::Ordering::Equal;
            };
            b.y.total_cmp(&a.y).then_with(|| a.x.total_cmp(&b.x))
        });
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for (index, before) in sources.into_iter().enumerate() {
            let number = i64::try_from(index)
                .ok()
                .and_then(|index| index.checked_mul(request.step))
                .and_then(|offset| request.start.checked_add(offset))
                .ok_or_else(|| {
                    refusal(
                        "tcount.overflow",
                        "the requested start/step/index numbering overflows i64",
                    )
                })?;
            let replacement = format!("{}{}{}", request.prefix, number, request.suffix);
            let SemanticGeometry::Text { text, .. } = &before.geometry else {
                return Err(CadError::Invariant("TCOUNT lost its text source".into()));
            };
            if *text == replacement {
                continue;
            }
            let mut after = before.clone();
            if let SemanticGeometry::Text { text, .. } = &mut after.geometry {
                *text = replacement;
            }
            planned.push((before, after));
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                "tcount.unchanged",
                "TCOUNT: every selected text already carries its number; nothing was written"
                    .into(),
                Vec::new(),
            ));
        }
        let (objects, change_set) = commit_text_updates(
            self,
            document_id,
            candidate,
            &planned,
            "text count",
            "text count",
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.tcount".into(),
                message: format!(
                    "TCOUNT: replaced {} text entities with start {} step {} in reading order \
                     (AutoCAD per-item dialog modes are NOT implemented)",
                    planned.len(),
                    request.start,
                    request.step
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
