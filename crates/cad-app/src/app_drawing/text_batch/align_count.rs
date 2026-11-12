//! `TEXTALIGN` (15 alignment modes and `DISTRIBUTE` gap spacing from estimated
//! extents) and `TCOUNT` (deterministic reading-order numbering).

use super::shared::*;
use super::*;

impl Application {
    /// `TEXTALIGN <mode>`: move the selected text to one common
    /// edge/centre measured from the estimated extents.
    pub(crate) fn text_align(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let edge = match &command.payload {
            CommandPayload::TextAlignEdges(request) => request.edge,
            CommandPayload::TextAlignSpacing(request) => {
                return self.text_align_spacing(session, command, request.spacing);
            }
            _ => {
                return Err(refusal(
                    "textalign.invalid_payload",
                    "TEXTALIGN needs a TextAlignEdges { edge } or TextAlignSpacing { spacing } \
                     payload (type `TEXTALIGN LEFT|RIGHT|CENTER|TOP|MIDDLE|BOTTOM|TOPLEFT|…` or \
                     `TEXTALIGN DISTRIBUTE [X|Y]`)",
                ));
            }
        };
        let document_id = command.document;
        let (candidate, targets) = self.text_align_targets(session, document_id, "textalign")?;
        if targets.len() < 2 {
            return Err(refusal(
                "textalign.needs_two",
                format!(
                    "TEXTALIGN needs at least two Text/MText targets, got {}",
                    targets.len()
                ),
            ));
        }
        let min_x = targets
            .iter()
            .map(|(_, bounds)| bounds.min_x)
            .fold(f64::INFINITY, f64::min);
        let max_x = targets
            .iter()
            .map(|(_, bounds)| bounds.max_x)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = targets
            .iter()
            .map(|(_, bounds)| bounds.min_y)
            .fold(f64::INFINITY, f64::min);
        let max_y = targets
            .iter()
            .map(|(_, bounds)| bounds.max_y)
            .fold(f64::NEG_INFINITY, f64::max);
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for (source, bounds) in targets {
            let mut after = source.clone();
            let mut moved = false;
            if let Some((current, target)) = align_x(edge, &bounds, min_x, max_x) {
                moved |= shift_text(&mut after, current, target, 0.0);
            }
            if let Some((current, target)) = align_y(edge, &bounds, min_y, max_y) {
                moved |= shift_text(&mut after, current, target, 1.0);
            }
            if moved {
                planned.push((source, after));
            }
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                "textalign.unchanged",
                format!(
                    "TEXTALIGN {:?}: every target already lies on the common edge/centre; \
                     nothing was written",
                    edge
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
                    edge,
                    planned.len()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `TEXTALIGN DISTRIBUTE [X|Y]`: equal gaps between consecutive estimated
    /// extents; the first and last extents stay fixed.
    fn text_align_spacing(
        &mut self,
        session: &SessionState,
        command: &Command,
        spacing: TextAlignSpacing,
    ) -> CadResult<CommandOutcome> {
        let axis = match spacing {
            TextAlignSpacing::Distribute(axis) => axis,
            TextAlignSpacing::SetSpacing => {
                return Err(refusal(
                    "textalign.set_spacing_unsupported",
                    "TEXTALIGN SETSPACING needs a numeric spacing value and per-object \
                     bases; only `TEXTALIGN DISTRIBUTE [X|Y]` is implemented",
                ));
            }
            TextAlignSpacing::CurrentVertical => {
                return Err(refusal(
                    "textalign.current_vertical_unsupported",
                    "TEXTALIGN CURRENTVERTICAL is not implemented; only `TEXTALIGN DISTRIBUTE \
                     [X|Y]` is implemented",
                ));
            }
            TextAlignSpacing::CurrentHorizontal => {
                return Err(refusal(
                    "textalign.current_horizontal_unsupported",
                    "TEXTALIGN CURRENTHORIZONTAL is not implemented; only `TEXTALIGN DISTRIBUTE \
                     [X|Y]` is implemented",
                ));
            }
        };
        let document_id = command.document;
        let (candidate, targets) = self.text_align_targets(session, document_id, "textalign")?;
        if targets.len() < 3 {
            return Err(refusal(
                "textalign.distribute_needs_three",
                format!(
                    "TEXTALIGN DISTRIBUTE needs at least three Text/MText targets, got {}",
                    targets.len()
                ),
            ));
        }
        let mut order: Vec<usize> = (0..targets.len()).collect();
        match axis {
            TextAlignAxis::Y => order.sort_by(|&left, &right| {
                let (left_entity, left_bounds) = &targets[left];
                let (right_entity, right_bounds) = &targets[right];
                right_bounds
                    .max_y
                    .total_cmp(&left_bounds.max_y)
                    .then_with(|| left_bounds.min_x.total_cmp(&right_bounds.min_x))
                    .then_with(|| left_entity.id.0.cmp(&right_entity.id.0))
            }),
            TextAlignAxis::X => order.sort_by(|&left, &right| {
                let (left_entity, left_bounds) = &targets[left];
                let (right_entity, right_bounds) = &targets[right];
                left_bounds
                    .min_x
                    .total_cmp(&right_bounds.min_x)
                    .then_with(|| right_bounds.max_y.total_cmp(&left_bounds.max_y))
                    .then_with(|| left_entity.id.0.cmp(&right_entity.id.0))
            }),
        }
        let count = targets.len();
        // Free span: first leading edge to last trailing edge, minus every
        // extent. `count - 1` gaps, hence one gap per interval.
        let free = match axis {
            TextAlignAxis::Y => {
                let first_top = targets[order[0]].1.max_y;
                let last_bottom = targets[order[count - 1]].1.min_y;
                let total: f64 = order.iter().map(|&index| targets[index].1.height()).sum();
                (first_top - last_bottom) - total
            }
            TextAlignAxis::X => {
                let first_left = targets[order[0]].1.min_x;
                let last_right = targets[order[count - 1]].1.max_x;
                let total: f64 = order.iter().map(|&index| targets[index].1.width()).sum();
                (last_right - first_left) - total
            }
        };
        let gap = free / (count - 1) as f64;
        if !gap.is_finite() {
            return Err(refusal(
                "textalign.degenerate",
                "TEXTALIGN DISTRIBUTE produced a non-finite gap from the estimated extents",
            ));
        }
        // Walk the reading order, carrying the trailing edge of the previously
        // placed extent. The first and last extents are never moved.
        let mut previous_edge = match axis {
            TextAlignAxis::Y => targets[order[0]].1.min_y,
            TextAlignAxis::X => targets[order[0]].1.max_x,
        };
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for (position, &index) in order.iter().enumerate().skip(1) {
            let (source, bounds) = &targets[index];
            let (current, target) = match axis {
                TextAlignAxis::Y => (bounds.max_y, previous_edge - gap),
                TextAlignAxis::X => (bounds.min_x, previous_edge + gap),
            };
            // The last extent is fixed; its computed target is the validation
            // invariant of the free-span arithmetic, not a write.
            if position + 1 < count {
                let mut after = source.clone();
                if shift_text(
                    &mut after,
                    current,
                    target,
                    if axis == TextAlignAxis::Y { 1.0 } else { 0.0 },
                ) {
                    planned.push((source.clone(), after));
                }
            }
            previous_edge = match axis {
                TextAlignAxis::Y => target - bounds.height(),
                TextAlignAxis::X => target + bounds.width(),
            };
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                "textalign.unchanged",
                format!(
                    "TEXTALIGN DISTRIBUTE {axis:?}: the gaps are already equal; nothing was \
                     written"
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
                    "TEXTALIGN DISTRIBUTE {axis:?}: moved {} interior text entities to equal \
                     gaps ({} targets, estimated bounds, first and last fixed)",
                    planned.len(),
                    count
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// The selected directly addressable model-space Text/MText targets with one
    /// estimated bounds each (deduplicated by entity id, first appearance wins).
    ///
    /// A non-text or unaddressable target refuses the whole command; an empty
    /// selection is `<prefix>.needs_selection`.
    fn text_align_targets(
        &self,
        session: &SessionState,
        document_id: DocumentId,
        prefix: &str,
    ) -> CadResult<(DrawingDatabase, Vec<(DbEntity, EstimatedTextBounds)>)> {
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                &format!("{prefix}.needs_selection"),
                "TEXTALIGN needs at least two selected Text/MText entities",
            ));
        }
        let candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let mut targets = Vec::with_capacity(references.len());
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            let entity = resolve_model_entity(&candidate, document_id, reference, prefix)?;
            require_text(prefix, &entity)?;
            if seen.insert(entity.id) {
                let bounds = estimate_text_bounds(&entity, prefix)?;
                targets.push((entity, bounds));
            }
        }
        Ok((candidate, targets))
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

/// The horizontal anchor `(current, target)` a mode aligns, or `None` when the
/// mode leaves X untouched.
///
/// `min_x`/`max_x` are the extremes of the selection's union estimated box.
fn align_x(
    edge: TextAlignEdge,
    bounds: &EstimatedTextBounds,
    min_x: f64,
    max_x: f64,
) -> Option<(f64, f64)> {
    match edge {
        TextAlignEdge::Left
        | TextAlignEdge::TopLeft
        | TextAlignEdge::MiddleLeft
        | TextAlignEdge::BottomLeft => Some((bounds.min_x, min_x)),
        TextAlignEdge::Right
        | TextAlignEdge::TopRight
        | TextAlignEdge::MiddleRight
        | TextAlignEdge::BottomRight => Some((bounds.max_x, max_x)),
        TextAlignEdge::Center
        | TextAlignEdge::TopCenter
        | TextAlignEdge::MiddleCenter
        | TextAlignEdge::BottomCenter => Some((bounds.center().0, (min_x + max_x) * 0.5)),
        TextAlignEdge::Top | TextAlignEdge::Middle | TextAlignEdge::Bottom => None,
    }
}

/// The vertical anchor `(current, target)` a mode aligns, or `None` when the
/// mode leaves Y untouched.
fn align_y(
    edge: TextAlignEdge,
    bounds: &EstimatedTextBounds,
    min_y: f64,
    max_y: f64,
) -> Option<(f64, f64)> {
    match edge {
        TextAlignEdge::Top
        | TextAlignEdge::TopLeft
        | TextAlignEdge::TopCenter
        | TextAlignEdge::TopRight => Some((bounds.max_y, max_y)),
        TextAlignEdge::Bottom
        | TextAlignEdge::BottomLeft
        | TextAlignEdge::BottomCenter
        | TextAlignEdge::BottomRight => Some((bounds.min_y, min_y)),
        TextAlignEdge::Middle
        | TextAlignEdge::MiddleLeft
        | TextAlignEdge::MiddleCenter
        | TextAlignEdge::MiddleRight => Some((bounds.center().1, (min_y + max_y) * 0.5)),
        TextAlignEdge::Left | TextAlignEdge::Right | TextAlignEdge::Center => None,
    }
}
