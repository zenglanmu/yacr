//! `MLEADERALIGN`, `MLEADEREDIT` and `MLEADERCOLLECT`: real 2D edits of the
//! stored MULTILEADER model.
//!
//! All three commands use the **existing** `MultiLeaderGeometry` definition
//! (style snapshot, `MLeaderText`, `MLeaderRoot`s, `MLeaderLine`s): no new
//! geometry variant is introduced. Each edit rebuilds the definition through the
//! single constructor ([`MultiLeaderGeometry::new`]), which re-derives the
//! display from `style + text + roots + lines`, so a stored display can never
//! disagree with the definition.
//!
//! ## `MLEADERALIGN <LEFT|RIGHT|CENTER> [spacing]`
//!
//! The current selection is the object input (at least two model-space
//! MULTILEADERs with text content). The target X is the minimum (`LEFT`), maximum
//! (`RIGHT`) or the midpoint of the two extremes (`CENTER`) of the selected
//! leaders' **existing** text X positions. With an explicit `spacing`, the texts
//! are additionally re-spaced top-to-bottom at that pitch, anchored on the
//! topmost current text Y; without it, every current Y is kept.
//!
//! Each moved leader keeps its **arrow position** (the first vertex of every
//! leader line) fixed and translates the annotation: the remaining leader
//! vertices, every root's landing point and the text location all move by the
//! same delta. That is a real leader reshape, not a whole-entity move.
//!
//! ## `MLEADEREDIT ADD <x> <y>` / `MLEADEREDIT REMOVE <index>`
//!
//! `ADD` appends one leader line to the first root, from the typed arrow position
//! to that root's existing dogleg vertex (the last vertex of its first line), so
//! the derived dogleg is exactly the same as the sibling lines'. `REMOVE` drops
//! one line by its 0-based index and refuses to empty a root (a root's leader
//! line list is what the display and the export contract require).
//!
//! ## `MLEADERCOLLECT`
//!
//! Collects two or more selected MULTILEADERs into **one**: all roots and leader
//! lines are concatenated, the annotation strings are joined with `\P` (MTEXT's
//! paragraph break) and the first leader's style snapshot/plane/attachment is
//! used. Every collected root's landing point is retargeted to the base text
//! position, because a multileader carries exactly one annotation. Leaders that disagree on style, plane or carry unmodelled reserved data
//! are refused rather than silently losing those fields. The sources are deleted
//! and the collected leader inserted in one transaction / one undo step.
//!
//! Every refusal is named (`mleaderalign.*`, `mleaderedit.*`,
//! `mleadercollect.*`), happens before anything is staged, and the whole edit is
//! all-or-nothing.

use super::*;
use crate::app_extended_w9_reports::text_explode::{replace_entities, ProducedEntity};
use crate::command_payload::{MLeaderAlignMode, MLeaderEditAction};
use cad_db::DbEntity;
use cad_domain::{
    MLeaderContent, MLeaderLine, MLeaderReserved, MLeaderText, MultiLeaderGeometry,
    MultiLeaderParts,
};

/// The relative tolerance for coincidence and coplanarity tests.
const MLEADER_EDIT_EPS: f64 = 1e-9;

/// A named invalid-input refusal (`<prefix>.<code>: …`).
fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

/// A named capability/subset refusal (`<prefix>.<code>: …`).
fn unsupported(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{prefix}.{code}: {detail}"))
}

/// One selected leader, re-read from the drawing.
struct ResolvedLeader {
    entity: DbEntity,
    leader: MultiLeaderGeometry,
}

impl Application {
    /// `MLEADERALIGN`: align / space the selected MULTILEADERs' annotation text.
    ///
    /// Refusals (named, nothing written): `mleaderalign.payload`,
    /// `mleaderalign.selection`, `mleaderalign.not_addressable`,
    /// `mleaderalign.target_missing`, `mleaderalign.space`,
    /// `mleaderalign.not_mleader`, `mleaderalign.content`,
    /// `mleaderalign.reserved_breaks`, `mleaderalign.plane`,
    /// `mleaderalign.spacing`. No-op: `mleaderalign.unchanged` (no transaction).
    pub(crate) fn mleader_align(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::MLeaderAlign(payload) = &command.payload else {
            return Err(invalid(
                "mleaderalign",
                "payload",
                "MLEADERALIGN needs a MLeaderAlign { mode, spacing } payload; the current \
                 selection is the object input",
            ));
        };
        let leaders = self.resolve_selected_leaders(session, command, "mleaderalign")?;
        if leaders.len() < 2 {
            return Err(invalid(
                "mleaderalign",
                "selection",
                format!(
                    "MLEADERALIGN needs at least two selected MULTILEADERs, got {}",
                    leaders.len()
                ),
            ));
        }
        for resolved in &leaders {
            if resolved.leader.content() != MLeaderContent::Text {
                return Err(unsupported(
                    "mleaderalign",
                    "content",
                    format!(
                        "entity {} carries no text content, so it has no annotation to align",
                        resolved.entity.id.0
                    ),
                ));
            }
            if has_reserved_breaks(&resolved.leader) {
                return Err(unsupported(
                    "mleaderalign",
                    "reserved_breaks",
                    format!(
                        "entity {} carries stored leader break pairs; reshapng the annotation \
                         would leave them stale",
                        resolved.entity.id.0
                    ),
                ));
            }
        }
        // A professional column lives on one drawing plane; mixing elevations
        // would align X/Y across planes, which the plane frame cannot express.
        let plane_z = leaders[0].leader.text().location.z;
        for resolved in &leaders[1..] {
            let z = resolved.leader.text().location.z;
            if (z - plane_z).abs() > MLEADER_EDIT_EPS * (1.0 + plane_z.abs()) {
                return Err(unsupported(
                    "mleaderalign",
                    "plane",
                    format!(
                        "entity {} sits at Z {z}, not at the selection's Z {plane_z}; aligning \
                         across planes is not representable",
                        resolved.entity.id.0
                    ),
                ));
            }
        }
        let target_x = match payload.mode {
            MLeaderAlignMode::Left => leaders
                .iter()
                .map(|resolved| resolved.leader.text().location.x)
                .fold(f64::INFINITY, f64::min),
            MLeaderAlignMode::Right => leaders
                .iter()
                .map(|resolved| resolved.leader.text().location.x)
                .fold(f64::NEG_INFINITY, f64::max),
            MLeaderAlignMode::Center => {
                let min = leaders
                    .iter()
                    .map(|resolved| resolved.leader.text().location.x)
                    .fold(f64::INFINITY, f64::min);
                let max = leaders
                    .iter()
                    .map(|resolved| resolved.leader.text().location.x)
                    .fold(f64::NEG_INFINITY, f64::max);
                (min + max) * 0.5
            }
        };
        if !target_x.is_finite() {
            return Err(invalid(
                "mleaderalign",
                "spacing",
                "the selected text positions do not define a finite alignment column",
            ));
        }
        // Top-to-bottom order for spacing; ties keep selection order.
        let mut order: Vec<usize> = (0..leaders.len()).collect();
        order.sort_by(|&left, &right| {
            leaders[right]
                .leader
                .text()
                .location
                .y
                .total_cmp(&leaders[left].leader.text().location.y)
        });
        let spacing = match payload.spacing {
            Some(value) => {
                if !value.is_finite() || value <= 0.0 {
                    return Err(invalid(
                        "mleaderalign",
                        "spacing",
                        format!("the vertical spacing must be finite and positive, got {value}"),
                    ));
                }
                Some(value)
            }
            None => None,
        };
        let anchor_y = leaders[order[0]].leader.text().location.y;
        let mut row_y = vec![f64::NAN; leaders.len()];
        for (rank, &index) in order.iter().enumerate() {
            row_y[index] = match spacing {
                Some(pitch) => anchor_y - pitch * rank as f64,
                None => leaders[index].leader.text().location.y,
            };
        }

        let mut targets: Vec<(DbEntity, DbEntity)> = Vec::new();
        for (index, resolved) in leaders.iter().enumerate() {
            let current = resolved.leader.text().location;
            let target = Point3 {
                x: target_x,
                y: row_y[index],
                z: current.z,
            };
            if (target.x - current.x).abs() <= MLEADER_EDIT_EPS
                && (target.y - current.y).abs() <= MLEADER_EDIT_EPS
            {
                continue;
            }
            let leader = move_annotation(&resolved.leader, target)?;
            let mut after = resolved.entity.clone();
            after.geometry = SemanticGeometry::MultiLeader(Box::new(leader));
            targets.push((resolved.entity.clone(), after));
        }
        if targets.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "mleaderalign.unchanged".into(),
                    message: format!(
                        "MLEADERALIGN {}: every selected leader already sits on the {} column; \
                         no transaction",
                        payload.mode.token(),
                        target_x
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let count = targets.len();
        let mut outcome = self.update_entities(command, targets, "mleaderalign")?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "mleaderalign.applied".into(),
            message: format!(
                "MLEADERALIGN {}: aligned {count} leader(s) on X {target_x}{}",
                payload.mode.token(),
                match spacing {
                    Some(pitch) => format!(" and re-spaced them at pitch {pitch}"),
                    None => String::new(),
                }
            ),
        });
        Ok(outcome)
    }

    /// `MLEADEREDIT`: add or remove one leader line on every selected leader.
    ///
    /// Refusals (named, nothing written): `mleaderedit.payload`,
    /// `mleaderedit.selection`, `mleaderedit.not_addressable`,
    /// `mleaderedit.target_missing`, `mleaderedit.space`,
    /// `mleaderedit.not_mleader`, `mleaderedit.arrow`, `mleaderedit.degenerate`,
    /// `mleaderedit.path_type`, `mleaderedit.index`, `mleaderedit.last_line`.
    pub(crate) fn mleader_edit(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::MLeaderEdit(payload) = &command.payload else {
            return Err(invalid(
                "mleaderedit",
                "payload",
                "MLEADEREDIT needs an MLeaderEdit { action } payload; the current selection is \
                 the object input",
            ));
        };
        let leaders = self.resolve_selected_leaders(session, command, "mleaderedit")?;
        if leaders.is_empty() {
            return Err(invalid(
                "mleaderedit",
                "selection",
                "MLEADEREDIT needs at least one selected MULTILEADER",
            ));
        }
        let mut targets: Vec<(DbEntity, DbEntity)> = Vec::with_capacity(leaders.len());
        for resolved in &leaders {
            let leader = match &payload.action {
                MLeaderEditAction::Add { arrow } => add_leader_line(&resolved.leader, *arrow)?,
                MLeaderEditAction::Remove { line_index } => {
                    remove_leader_line(&resolved.leader, *line_index)?
                }
            };
            let mut after = resolved.entity.clone();
            after.geometry = SemanticGeometry::MultiLeader(Box::new(leader));
            targets.push((resolved.entity.clone(), after));
        }
        let mut outcome = self.update_entities(command, targets, "mleaderedit")?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "mleaderedit.applied".into(),
            message: format!(
                "MLEADEREDIT {}: rewritten {} leader(s)",
                match payload.action {
                    MLeaderEditAction::Add { .. } => "ADD",
                    MLeaderEditAction::Remove { .. } => "REMOVE",
                },
                leaders.len()
            ),
        });
        Ok(outcome)
    }

    /// `MLEADERCOLLECT`: merge the selected MULTILEADERs into one.
    ///
    /// Refusals (named, nothing written): `mleadercollect.payload`,
    /// `mleadercollect.selection`, `mleadercollect.not_addressable`,
    /// `mleadercollect.target_missing`, `mleadercollect.space`,
    /// `mleadercollect.not_mleader`, `mleadercollect.content`,
    /// `mleadercollect.style`, `mleadercollect.plane`, `mleadercollect.reserved`.
    pub(crate) fn mleader_collect(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "mleadercollect",
                "payload",
                "MLEADERCOLLECT takes no payload; the current selection is the object input",
            ));
        }
        let leaders = self.resolve_selected_leaders(session, command, "mleadercollect")?;
        if leaders.len() < 2 {
            return Err(invalid(
                "mleadercollect",
                "selection",
                format!(
                    "MLEADERCOLLECT needs at least two selected MULTILEADERs, got {}",
                    leaders.len()
                ),
            ));
        }
        let base = &leaders[0].leader;
        for resolved in &leaders {
            let leader = &resolved.leader;
            if leader.content() != MLeaderContent::Text || leader.text().string.trim().is_empty() {
                return Err(unsupported(
                    "mleadercollect",
                    "content",
                    format!(
                        "entity {} carries no text content; a collected leader needs one \
                         annotation",
                        resolved.entity.id.0
                    ),
                ));
            }
            if leader.style_name() != base.style_name()
                || leader.override_mask() != base.override_mask()
            {
                return Err(unsupported(
                    "mleadercollect",
                    "style",
                    format!(
                        "entity {} uses a different leader style/override mask; collecting \
                         would silently drop one style snapshot",
                        resolved.entity.id.0
                    ),
                ));
            }
            if leader.plane() != base.plane() {
                return Err(unsupported(
                    "mleadercollect",
                    "plane",
                    format!(
                        "entity {} lies on a different plane frame; one collected leader has \
                         one plane",
                        resolved.entity.id.0
                    ),
                ));
            }
            if !reserved_is_empty(leader.reserved()) {
                return Err(unsupported(
                    "mleadercollect",
                    "reserved",
                    format!(
                        "entity {} carries unmodelled reserved data (breaks/background/columns); \
                         collecting would drop it",
                        resolved.entity.id.0
                    ),
                ));
            }
        }

        let mut roots = Vec::new();
        let mut lines = Vec::new();
        let mut strings = Vec::with_capacity(leaders.len());
        for resolved in &leaders {
            let leader = &resolved.leader;
            let root_offset = roots.len();
            for root in leader.roots() {
                let mut root = root.clone();
                root.leader_index = roots.len() as i32;
                // Every collected leader line ends on the one annotation: the
                // root's landing point moves to the base text position, and the
                // derived dogleg is rebuilt from that root's own last vertex. A
                // multileader has exactly one annotation text, so leaving the
                // other roots' landings elsewhere would draw leader lines to
                // nothing.
                root.landing_point = base.text().location;
                roots.push(root);
            }
            for line in leader.lines() {
                let mut line = line.clone();
                line.root_index += root_offset;
                lines.push(line);
            }
            strings.push(leader.text().string.clone());
        }
        let base_text = base.text();
        let text = MLeaderText {
            string: strings.join("\\P"),
            ..base_text.clone()
        };
        let root_count = roots.len();
        let line_count = lines.len();
        let parts = MultiLeaderParts {
            style: base.style().clone(),
            style_name: base.style_name().to_string(),
            override_mask: base.override_mask(),
            content: MLeaderContent::Text,
            text,
            roots,
            lines,
            reserved: MLeaderReserved::default(),
            plane: base.plane(),
        };
        let collected = MultiLeaderGeometry::new(parts)?;
        let sources: Vec<DbEntity> = leaders
            .iter()
            .map(|resolved| resolved.entity.clone())
            .collect();
        let produced = vec![ProducedEntity {
            geometry: SemanticGeometry::MultiLeader(Box::new(collected)),
            layer: sources[0].layer,
            space: sources[0].space.clone(),
        }];
        let count = sources.len();
        let mut outcome = replace_entities(
            self,
            command,
            sources,
            produced,
            "mleadercollect",
            "mleadercollect.applied",
        )?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "mleadercollect.representation".into(),
            message: format!(
                "MLEADERCOLLECT: merged {count} MULTILEADERs into one annotation with {} roots \
                 and {} leader line(s); the text strings are joined with MTEXT's \\P paragraph \
                 break, every root landing is retargeted to the base text position and the \
                 first leader's style snapshot is kept",
                root_count, line_count
            ),
        });
        Ok(outcome)
    }

    /// Stage one `update_entity` per target in **one** transaction / one undo step.
    fn update_entities(
        &mut self,
        command: &Command,
        targets: Vec<(DbEntity, DbEntity)>,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
            for (_, after) in &targets {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(targets.len());
        let mut objects = Vec::with_capacity(targets.len());
        for (before, _) in &targets {
            let after = candidate.entity(before.id).cloned().ok_or_else(|| {
                CadError::Invariant(format!("{label}: an edited leader vanished after commit"))
            })?;
            patches.push(drawing_patch(before.id, Some(before.clone()), Some(after)));
            objects.push(ObjectId(before.id.0));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches,
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: Vec::new(),
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve every selected ref as a model-space MULTILEADER.
impl Application {
    fn resolve_selected_leaders(
        &self,
        session: &SessionState,
        command: &Command,
        prefix: &str,
    ) -> CadResult<Vec<ResolvedLeader>> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let drawing = document.drawing.as_ref();
        let references = session.selection.refs();
        let mut resolved = Vec::with_capacity(references.len());
        for reference in references {
            resolved.push(resolve_leader(drawing, command, reference, prefix)?);
        }
        Ok(resolved)
    }
}

/// Re-read one selected ref as a model-space MULTILEADER.
fn resolve_leader(
    drawing: &cad_db::DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
    prefix: &str,
) -> CadResult<ResolvedLeader> {
    if reference.document != command.document {
        return Err(invalid(
            prefix,
            "not_addressable",
            format!(
                "the ref for entity {} belongs to another document",
                reference.entity.0
            ),
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(unsupported(
            prefix,
            "not_addressable",
            format!(
                "entity {} is referenced through a block instance or a sub-element; pick the \
                 MULTILEADER entity itself",
                reference.entity.0
            ),
        ));
    }
    let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
        invalid(
            prefix,
            "target_missing",
            format!(
                "entity {} does not exist in this drawing",
                reference.entity.0
            ),
        )
    })?;
    if entity.space != SpaceId::Model {
        return Err(unsupported(
            prefix,
            "space",
            format!(
                "entity {} is not in model space; this edit is model-space only",
                reference.entity.0
            ),
        ));
    }
    let leader = match &entity.geometry {
        SemanticGeometry::MultiLeader(leader) => leader.as_ref().clone(),
        other => {
            return Err(unsupported(
                prefix,
                "not_mleader",
                format!(
                    "entity {} is a {} entity, not a MULTILEADER",
                    reference.entity.0,
                    class_name(other)
                ),
            ))
        }
    };
    Ok(ResolvedLeader { entity, leader })
}

/// Rebuild a leader with its annotation translated to `target`, keeping every
/// leader line's first (arrow) vertex fixed.
fn move_annotation(leader: &MultiLeaderGeometry, target: Point3) -> CadResult<MultiLeaderGeometry> {
    let current = leader.text().location;
    let delta = cad_geometry::sub(target, current);
    let mut parts = parts_of(leader);
    for root in &mut parts.roots {
        root.landing_point = cad_geometry::add(root.landing_point, delta);
    }
    for line in &mut parts.lines {
        for point in line.points.iter_mut().skip(1) {
            *point = cad_geometry::add(*point, delta);
        }
    }
    parts.text.location = target;
    MultiLeaderGeometry::new(parts)
}

/// Append one leader line to the first root, ending on that root's dogleg
/// vertex.
fn add_leader_line(leader: &MultiLeaderGeometry, arrow: Point3) -> CadResult<MultiLeaderGeometry> {
    if !is_finite_point(arrow) {
        return Err(invalid(
            "mleaderedit",
            "arrow",
            "the new leader arrow position must be finite",
        ));
    }
    if leader.style().path_type == cad_domain::MLeaderPathType::Invisible {
        return Err(unsupported(
            "mleaderedit",
            "path_type",
            "the resolved leader style draws no leader line (invisible path type), so a new \
             line would never be displayed",
        ));
    }
    let first_root = leader
        .lines()
        .iter()
        .find(|line| line.root_index == 0)
        .ok_or_else(|| {
            unsupported(
                "mleaderedit",
                "no_line",
                "the multileader has no leader line on its first root to extend",
            )
        })?;
    let connection = *first_root.points.last().ok_or_else(|| {
        unsupported(
            "mleaderedit",
            "no_line",
            "the multileader's first leader line has no vertices",
        )
    })?;
    if cad_geometry::distance(arrow, connection) <= MLEADER_EDIT_EPS {
        return Err(invalid(
            "mleaderedit",
            "degenerate",
            "the new arrow position coincides with the root's dogleg vertex",
        ));
    }
    let mut parts = parts_of(leader);
    let next_index = parts
        .lines
        .iter()
        .filter(|line| line.root_index == 0)
        .map(|line| line.index)
        .max()
        .unwrap_or(-1)
        + 1;
    parts.lines.push(MLeaderLine {
        root_index: 0,
        index: next_index,
        points: vec![arrow, connection],
        path_type: first_root.path_type,
        // No per-line overrides: the root/style values apply, exactly like the
        // sibling line.
        override_flags: 0,
        arrowhead_size: leader.style().arrowhead_size,
        arrowhead_handle: None,
    });
    MultiLeaderGeometry::new(parts)
}

/// Remove one leader line by its 0-based index, refusing to empty a root.
fn remove_leader_line(
    leader: &MultiLeaderGeometry,
    line_index: usize,
) -> CadResult<MultiLeaderGeometry> {
    let Some(line) = leader.lines().get(line_index) else {
        return Err(invalid(
            "mleaderedit",
            "index",
            format!(
                "leader line index {line_index} is out of range; the leader stores {} line(s)",
                leader.lines().len()
            ),
        ));
    };
    let root_index = line.root_index;
    let root_lines = leader
        .lines()
        .iter()
        .filter(|line| line.root_index == root_index)
        .count();
    if root_lines <= 1 {
        return Err(unsupported(
            "mleaderedit",
            "last_line",
            format!(
                "leader line {line_index} is the only line of root {root_index}; a root needs at \
                 least one line, so REMOVE cannot empty it"
            ),
        ));
    }
    let mut parts = parts_of(leader);
    parts.lines.remove(line_index);
    MultiLeaderGeometry::new(parts)
}

/// The definition parts of one stored leader (every field the constructor reads).
fn parts_of(leader: &MultiLeaderGeometry) -> MultiLeaderParts {
    MultiLeaderParts {
        style: leader.style().clone(),
        style_name: leader.style_name().to_string(),
        override_mask: leader.override_mask(),
        content: leader.content(),
        text: leader.text().clone(),
        roots: leader.roots().to_vec(),
        lines: leader.lines().to_vec(),
        reserved: leader.reserved().clone(),
        plane: leader.plane(),
    }
}

/// Whether the leader carries stored break pairs the reshape cannot move.
fn has_reserved_breaks(leader: &MultiLeaderGeometry) -> bool {
    leader
        .reserved()
        .roots
        .iter()
        .any(|root| !root.breaks.is_empty())
        || leader
            .reserved()
            .lines
            .iter()
            .any(|line| !line.breaks.is_empty() || !line.legacy_breaks.is_empty())
}

/// Whether the reserved bag carries anything at all.
fn reserved_is_empty(reserved: &MLeaderReserved) -> bool {
    reserved.roots.is_empty()
        && reserved.lines.is_empty()
        && reserved.background.is_none()
        && reserved.columns.is_none()
}
