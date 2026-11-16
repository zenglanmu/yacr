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
//! ## `MLEADEREDIT ADD <x> <y>` / `REMOVE <index>`
//!
//! `ADD` appends one leader line to the first root, from the typed arrow position
//! to that root's existing dogleg vertex (the last vertex of its first line), so
//! the derived dogleg is exactly the same as the sibling lines'. `REMOVE` drops
//! one line by its 0-based index. It may empty a root — that root then keeps its
//! landing point and dogleg length (`LEADER` `40`/`11`) and draws no leader line,
//! which the `LEADER{}` record can carry on export (no `LEADER_LINE{}` child).
//! The only structural floor is the multileader itself: its **last** remaining
//! line is refused by name (`mleaderedit.last_line`), because
//! [`MultiLeaderGeometry::new`] requires at least one line.
//!
//! ## `MLEADEREDIT PATHTYPE <index> LINE|SPLINE|INVISIBLE`
//!
//! Sets one line's **own** path type (`LEADER_LINE 170`) and stamps override word
//! (`93`) bit `1`. DXF carries that override per line and the exporter writes both
//! verbatim, so the requested path survives a reload instead of re-resolving from
//! the entity/style value. `INVISIBLE` is admitted: it is a real per-line DXF
//! state (the line draws no path and no arrowhead) and the display honours it.
//!
//! ## `MLEADEREDIT ARROWSIZE <index> <size>`
//!
//! Sets one line's **own** arrowhead size (`LEADER_LINE 40`) and stamps override
//! word bit `16`. The size must be finite and strictly positive
//! (`mleaderedit.arrowhead_size`); a stored per-line `0` has no unambiguous
//! meaning in the format — this display would draw no arrowhead, AutoCAD's own
//! reading is unverified — so it is refused rather than guessed.
//!
//! ## `MLEADEREDIT BYSTYLE <index> PATH|ARROW`
//!
//! Clears the line's override bit (`1` / `16`) and re-stamps the stored value from
//! the resolved style snapshot, so the entity/style value applies again and the
//! field never keeps a stale value. Without it the override word would be
//! one-way: a line forced to `SPLINE` could never follow its style again.
//!
//! Every action edits **one stored definition per selected leader** and stages all
//! of them in one transaction / one undo step; a refusal on any selected leader
//! refuses the whole command and writes nothing. An action whose requested state
//! a leader already stores is a declared no-op (`mleaderedit.unchanged`, no
//! transaction).
//!
//! The other per-line data this build stores is **not** editable here and is not
//! silently dropped, because it has no editable semantic model: per-line colour /
//! linetype / lineweight and the break records live in the opaque `reserved` bag as
//! verbatim source data, and the custom arrowhead **block** handle (`341`) names a
//! block this build cannot create or reference. The typed grammar therefore refuses
//! those tokens by name (`mleaderedit.argument`) instead of accepting a value it
//! could not write.
//!
//! ## `MLEADERCOLLECT`
//!
//! Collects two or more selected MULTILEADERs into **one**: all roots and leader
//! lines are concatenated, the annotation strings are joined with `\P` (MTEXT's
//! paragraph break) and the first leader's style snapshot/plane/attachment is
//! used. Every collected root's landing point is retargeted to the base text
//! position, because a multileader carries exactly one annotation. The sources
//! are deleted and the collected leader inserted in one transaction / one undo
//! step.
//!
//! A differing style, `90` override mask or plane frame is **accepted**, not
//! refused — collecting is what the command is for — and nothing is lost
//! silently:
//!
//! * the per-line **path type** (leader-line `170` with override word `93` bit
//!   `1`) and **arrowhead size** (`40` with bit `16`) of every collected line are
//!   carried as per-line overrides whenever they differ from the base snapshot,
//!   so those lines keep the appearance they had in their own leader;
//! * the fields with **no per-line group in DXF** (annotation text height
//!   `CONTEXT_DATA 41`, landing gap `145`, text style `343`, entity scale `45`)
//!   and the one plane frame (`110`/`111`/`112`/`297`) are adopted from the base
//!   leader, and the report names every adopted value and the entity it came
//!   from (`mleadercollect.style_adopted` / `mleadercollect.plane_adopted`);
//! * stored `reserved` data (break pairs, background fill, columns) still
//!   refuses by name: it is per-source data with no merge semantics, and writing
//!   a base root's break indices onto merged lines would be a lie.
//!
//! Every refusal is named (`mleaderalign.*`, `mleaderedit.*`,
//! `mleadercollect.*`), happens before anything is staged, and the whole edit is
//! all-or-nothing.
use super::mleader_collect::{
    effective_line_arrowhead, effective_line_path, same_measure, CollectAdoption,
};
use super::mleader_line_override::{
    clear_line_override, line_index_refusal, set_line_arrowhead_size, set_line_path_type,
};
use super::*;
use crate::app_extended_w9_reports::text_explode::{replace_entities, ProducedEntity};
use crate::command_payload::{MLeaderAlignMode, MLeaderEditAction};
use cad_db::DbEntity;
use cad_domain::{
    MLeaderContent, MLeaderLine, MLeaderReserved, MLeaderText, MultiLeaderGeometry,
    MultiLeaderParts,
};

/// The relative tolerance for coincidence and coplanarity tests.
pub(super) const MLEADER_EDIT_EPS: f64 = 1e-9;

/// A named invalid-input refusal (`<prefix>.<code>: …`).
pub(super) fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
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

    /// `MLEADEREDIT`: add, remove or re-override one leader line on every
    /// selected leader.
    ///
    /// Refusals (named, nothing written): `mleaderedit.payload`,
    /// `mleaderedit.selection`, `mleaderedit.not_addressable`,
    /// `mleaderedit.target_missing`, `mleaderedit.space`,
    /// `mleaderedit.not_mleader`, `mleaderedit.arrow`, `mleaderedit.degenerate`,
    /// `mleaderedit.path_type`, `mleaderedit.index`, `mleaderedit.last_line`,
    /// `mleaderedit.arrowhead_size`. No-op: `mleaderedit.unchanged` (no
    /// transaction).
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
        let mut unchanged = 0usize;
        for resolved in &leaders {
            // `None` is the declared no-op: the stored line already carries
            // exactly the requested state, so staging it would bump the revision
            // and the undo stack for nothing.
            let Some(leader) = edit_leader_lines(&resolved.leader, &payload.action)? else {
                unchanged += 1;
                continue;
            };
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
                    code: "mleaderedit.unchanged".into(),
                    message: format!(
                        "MLEADEREDIT {}: every selected leader already stores that value; no \
                         transaction",
                        payload.action.token()
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let rewritten = targets.len();
        let mut outcome = self.update_entities(command, targets, "mleaderedit")?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "mleaderedit.applied".into(),
            message: format!(
                "MLEADEREDIT {}: rewritten {rewritten} leader(s){}",
                payload.action.token(),
                if unchanged > 0 {
                    format!("; {unchanged} already stored that value")
                } else {
                    String::new()
                }
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
    /// `mleadercollect.reserved`.
    ///
    /// A differing style, override mask or plane frame is **adopted** from the
    /// first selected leader, and whatever the DXF format can carry per line
    /// (path type `170`, arrowhead size `40` behind override word `93`) is kept
    /// on the line itself; the rest is named in `mleadercollect.style_adopted`
    /// / `mleadercollect.plane_adopted`. See [`CollectAdoption`].
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
            if !reserved_is_empty(leader.reserved()) {
                return Err(unsupported(
                    "mleadercollect",
                    "reserved",
                    format!(
                        "entity {} carries unmodelled reserved data (breaks/background/columns); \
                         break pairs and columns are per-source records with no merge semantics, \
                         so collecting would drop them",
                        resolved.entity.id.0
                    ),
                ));
            }
        }

        let mut adoption = CollectAdoption::new(base);
        let mut carried_per_line = false;
        let mut roots = Vec::new();
        let mut lines = Vec::new();
        let mut strings = Vec::with_capacity(leaders.len());
        for (source_index, resolved) in leaders.iter().enumerate() {
            let leader = &resolved.leader;
            if source_index > 0 {
                adoption.note_style(leader, resolved.entity.id.0);
                adoption.note_plane(leader, resolved.entity.id.0);
            }
            let root_offset = roots.len();
            for root in leader.roots() {
                let mut root = root.clone();
                root.leader_index = roots.len() as i32;
                // Every collected leader line ends on the one annotation: the
                // root's landing point moves to the base text position, and the
                // derived dogleg is rebuilt from that root's own last vertex. A
                // multileader has exactly one annotation text, so leaving the
                // other roots' landings elsewhere would draw leader lines to
                // nothing. The root's own dogleg length/direction (LEADER `40` /
                // `11`) travel with the clone, so no root loses its dogleg.
                root.landing_point = base.text().location;
                roots.push(root);
            }
            for line in leader.lines() {
                // The per-line values this line actually draws with, resolved
                // the one way the display resolves them.
                let path = effective_line_path(leader, line);
                let arrowhead = effective_line_arrowhead(leader, line);
                let mut line = line.clone();
                line.root_index += root_offset;
                // DXF can carry both of these per line (170/40 behind override
                // word 93), so a difference from the base snapshot is preserved
                // instead of being adopted.
                if path != adoption.path {
                    line.path_type = path;
                    line.override_flags |= MLeaderLine::PATH_TYPE;
                    carried_per_line = true;
                }
                if !same_measure(arrowhead, adoption.arrowhead) {
                    if arrowhead.is_finite() {
                        line.arrowhead_size = arrowhead;
                        line.override_flags |= MLeaderLine::ARROWHEAD_SIZE;
                        carried_per_line = true;
                    } else {
                        // A non-finite source size cannot be written as an
                        // override; the merged line falls back to the base
                        // snapshot, and the report names it.
                        adoption.note_field(
                            "per-line arrowhead size (leader-line 40, non-finite source value)",
                            resolved.entity.id.0,
                        );
                    }
                }
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
        outcome
            .diagnostics
            .extend(adoption.report(carried_per_line));
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

/// Apply one typed action to one leader's stored line list.
///
/// `Ok(None)` is the declared **no-op**: the stored data already carries exactly
/// the requested state, so staging it would bump the revision and the undo stack
/// for nothing. Every refusal is an `Err` and leaves the leader untouched, so a
/// selection-wide edit stays all-or-nothing.
fn edit_leader_lines(
    leader: &MultiLeaderGeometry,
    action: &MLeaderEditAction,
) -> CadResult<Option<MultiLeaderGeometry>> {
    match action {
        MLeaderEditAction::Add { arrow } => add_leader_line(leader, *arrow).map(Some),
        MLeaderEditAction::Remove { line_index } => {
            remove_leader_line(leader, *line_index).map(Some)
        }
        MLeaderEditAction::SetPathType {
            line_index,
            path_type,
        } => set_line_path_type(leader, *line_index, *path_type),
        MLeaderEditAction::SetArrowheadSize { line_index, size } => {
            set_line_arrowhead_size(leader, *line_index, *size)
        }
        MLeaderEditAction::ClearOverride { line_index, field } => {
            clear_line_override(leader, *line_index, *field)
        }
    }
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

/// Remove one leader line by its 0-based index.
///
/// Removing a root's **last** line is allowed: that root keeps its landing point
/// and dogleg length (`LEADER` `40`/`11`) but draws no leader line, which is
/// exactly what the user asked for and what the `LEADER{}` record can carry on
/// export (a root with no `LEADER_LINE{}` child). The one structural floor is
/// the multileader itself: [`MultiLeaderGeometry::new`] requires at least one
/// leader line, so the leader's **only** remaining line is refused by name
/// (`mleaderedit.last_line`), and nothing is written.
fn remove_leader_line(
    leader: &MultiLeaderGeometry,
    line_index: usize,
) -> CadResult<MultiLeaderGeometry> {
    if leader.lines().get(line_index).is_none() {
        return Err(line_index_refusal(leader, line_index));
    }
    if leader.lines().len() <= 1 {
        return Err(unsupported(
            "mleaderedit",
            "last_line",
            format!(
                "leader line {line_index} is the multileader's only line; a MULTILEADER needs \
                 at least one leader line, so REMOVE cannot empty it"
            ),
        ));
    }
    let mut parts = parts_of(leader);
    parts.lines.remove(line_index);
    MultiLeaderGeometry::new(parts)
}

/// The definition parts of one stored leader (every field the constructor reads).
pub(super) fn parts_of(leader: &MultiLeaderGeometry) -> MultiLeaderParts {
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
