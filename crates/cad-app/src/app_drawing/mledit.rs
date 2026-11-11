//! `MLEDIT` / `-MLEDIT`: the **two-LINE subset** of multiline editing.
//!
//! ## Why a subset
//!
//! This build has no stored `AcDbMline` vertex/join model: `MLINE` creation
//! writes **two independent `LINE`s** at the style spacing
//! ([`super::draw2_batch::Application::mline`]), and an imported `AcDbMline` arrives as
//! one `Compound` entity whose per-element polylines carry no editable vertex
//! list. Everything AutoCAD's MLEDIT dialog expresses with caps, miters, joins,
//! fill and vertex add/remove therefore has no representation here.
//!
//! What *is* representable is the geometric clean-up of two lines at their
//! intersection, plus opening and breaking them. The current selection is the
//! object input (exactly two `LINE`s, in model space); the typed payload only
//! names the option and the pick point(s).
//!
//! ## The four clean-up spellings
//!
//! `MERGE`, `CLOSED`, `CROSS` and `TEE` all run the one representable clean-up:
//! each selected line's endpoint **nearest** the computed intersection is moved
//! onto it. A line that stops short is extended; a line that crosses the
//! intersection is trimmed back to it (the shorter overhang is removed). The
//! requested spelling is echoed in the report, and `mledit.residual` states what
//! the model cannot draw. Parallel (or coincident) lines have no single
//! intersection and are refused (`mledit.parallel`).
//!
//! ## OPEN
//!
//! `OPEN` splits **both** crossing lines at the intersection, so each line
//! becomes two `LINE`s. The two-LINE model carries no multiline width, so the
//! opened gap is zero-width; that is the declared residual (AutoCAD leaves a
//! gap the width of the multiline). The intersection must lie strictly inside
//! both segments (`mledit.open_range`).
//!
//! ## BREAK
//!
//! `BREAK <x1> <y1> <x2> <y2>` removes the band between the two picked points
//! from **two parallel lines**: each line is replaced by the two open segments
//! on either side of the band. Non-parallel lines are refused
//! (`mledit.break_parallel`) and so is a band that does not cut both lines
//! strictly inside their extents (`mledit.break_range`). The band is applied
//! through the picked points' projections onto each line, so it is perpendicular
//! to the pair's common direction, exactly like a multiline break.
//!
//! ## Write discipline
//!
//! Clean-up and OPEN/BREAK each stage **one transaction / one undo step**.
//! A clean-up that moves nothing opens no transaction and reports
//! `mledit.unchanged` (never a silent empty success). Refusals are named
//! (`mledit.<reason>`) and happen before anything is staged.

use super::*;
use crate::app_extended_w9_reports::text_explode::{replace_entities, ProducedEntity};
use crate::command_payload::MlEditMode;
use cad_db::DbEntity;
use cad_geometry::{cross, dot, scale, sub};

/// The relative tolerance for intersection and band tests.
const MLEDIT_EPS: f64 = 1e-9;

/// A named invalid-input refusal (`mledit.<code>: …`).
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("mledit.{code}: {detail}"))
}

/// A named capability/subset refusal (`mledit.<code>: …`).
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("mledit.{code}: {detail}"))
}

/// One selected line, re-read from the drawing.
struct SelectedLine {
    entity: DbEntity,
    start: Point3,
    end: Point3,
}

impl SelectedLine {
    fn direction(&self) -> Point3 {
        sub(self.end, self.start)
    }

    fn length(&self) -> f64 {
        cad_geometry::length(self.direction())
    }
}

impl Application {
    /// `MLEDIT` / `-MLEDIT`: run one two-LINE multiline edit.
    ///
    /// Refusals (named, nothing written): `mledit.payload`, `mledit.selection`,
    /// `mledit.not_addressable`, `mledit.target_missing`, `mledit.space`,
    /// `mledit.acdbmline_model`, `mledit.not_line`, `mledit.degenerate`,
    /// `mledit.parallel`, `mledit.no_intersection`, `mledit.open_range`,
    /// `mledit.break_parallel`, `mledit.break_range`. A clean-up that changes
    /// nothing is the no-op report `mledit.unchanged` (no transaction).
    pub(crate) fn mledit(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::MlEdit(payload) = &command.payload else {
            return Err(invalid(
                "payload",
                "MLEDIT needs an MlEdit { mode, first, second } payload; the current selection \
                 is the object input",
            ));
        };
        let references = session.selection.refs().to_vec();
        if references.len() != 2 {
            return Err(invalid(
                "selection",
                format!(
                    "MLEDIT needs exactly two selected LINEs, got {}",
                    references.len()
                ),
            ));
        }
        let (first, second) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            (
                resolve_line(document.drawing.as_ref(), command, &references[0])?,
                resolve_line(document.drawing.as_ref(), command, &references[1])?,
            )
        };
        match payload.mode {
            MlEditMode::Break => {
                let second_point = payload.second.ok_or_else(|| {
                    invalid(
                        "payload",
                        "MLEDIT BREAK needs two picked points (the band corners)",
                    )
                })?;
                self.mledit_break(command, &first, &second, payload.first, second_point)
            }
            MlEditMode::Open => self.mledit_open(command, &first, &second, payload.first),
            _ => self.mledit_close(command, payload.mode, &first, &second, payload.first),
        }
    }

    /// The one representable clean-up: move each line's nearest endpoint onto
    /// the intersection.
    fn mledit_close(
        &mut self,
        command: &Command,
        mode: MlEditMode,
        first: &SelectedLine,
        second: &SelectedLine,
        pick: Point3,
    ) -> CadResult<CommandOutcome> {
        if !is_finite_point(pick) {
            return Err(invalid(
                "no_intersection",
                "the picked crossing point is not finite",
            ));
        }
        let intersection = line_intersection(first, second)?;
        let mut targets: Vec<(DbEntity, DbEntity)> = Vec::new();
        for line in [first, second] {
            if let Some(after) = trim_or_extend(line, intersection)? {
                targets.push((line.entity.clone(), after));
            }
        }
        if targets.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![
                    Diagnostic {
                        object: None,
                        code: "mledit.unchanged".into(),
                        message: format!(
                            "MLEDIT {}: both selected lines already meet at ({}, {}); no \
                             transaction",
                            mode.token(),
                            intersection.x,
                            intersection.y
                        ),
                    },
                    residual(mode, intersection),
                ],
                measurement: None,
                point_report: None,
            });
        }
        let count = targets.len();
        let diagnostics = vec![
            Diagnostic {
                object: None,
                code: "mledit.applied".into(),
                message: format!(
                    "MLEDIT {}: moved {count} line endpoint(s) onto the intersection ({}, {})",
                    mode.token(),
                    intersection.x,
                    intersection.y
                ),
            },
            residual(mode, intersection),
        ];
        self.commit_updates(command, targets, "mledit", diagnostics)
    }

    /// `MLEDIT OPEN`: split both lines at the intersection.
    fn mledit_open(
        &mut self,
        command: &Command,
        first: &SelectedLine,
        second: &SelectedLine,
        pick: Point3,
    ) -> CadResult<CommandOutcome> {
        if !is_finite_point(pick) {
            return Err(invalid(
                "no_intersection",
                "the picked crossing point is not finite",
            ));
        }
        let intersection = line_intersection(first, second)?;
        // A split needs the intersection strictly inside both segments: an end
        // point would produce a zero-length piece, which is not a LINE.
        for line in [first, second] {
            if !strictly_inside(line, intersection) {
                return Err(invalid(
                    "open_range",
                    format!(
                        "MLEDIT OPEN needs the intersection inside both selected lines; entity \
                         {} meets it at an end point",
                        line.entity.id.0
                    ),
                ));
            }
        }
        let produced: Vec<ProducedEntity> = [first, second]
            .into_iter()
            .flat_map(|line| {
                [
                    ProducedEntity {
                        geometry: SemanticGeometry::Line {
                            start: line.start,
                            end: intersection,
                        },
                        layer: line.entity.layer,
                        space: line.entity.space.clone(),
                    },
                    ProducedEntity {
                        geometry: SemanticGeometry::Line {
                            start: intersection,
                            end: line.end,
                        },
                        layer: line.entity.layer,
                        space: line.entity.space.clone(),
                    },
                ]
            })
            .collect();
        let sources = vec![first.entity.clone(), second.entity.clone()];
        let mut outcome = replace_entities(
            self,
            command,
            sources,
            produced,
            "mledit open",
            "mledit.applied",
        )?;
        outcome
            .diagnostics
            .push(residual(MlEditMode::Open, intersection));
        Ok(outcome)
    }

    /// `MLEDIT BREAK`: remove the picked band from two parallel lines.
    fn mledit_break(
        &mut self,
        command: &Command,
        first: &SelectedLine,
        second: &SelectedLine,
        band_start: Point3,
        band_end: Point3,
    ) -> CadResult<CommandOutcome> {
        if !is_finite_point(band_start) || !is_finite_point(band_end) {
            return Err(invalid(
                "break_range",
                "the two band corners must be finite points",
            ));
        }
        let direction = first.direction();
        let length = first.length();
        if length <= MLEDIT_EPS {
            return Err(invalid(
                "degenerate",
                "the first selected line has no length",
            ));
        }
        let unit = scale(direction, 1.0 / length);
        // The second line must be parallel to the first: the multiline break
        // removes one band across the whole width.
        let second_length = second.length();
        if second_length <= MLEDIT_EPS {
            return Err(invalid(
                "degenerate",
                "the second selected line has no length",
            ));
        }
        let unit_second = scale(second.direction(), 1.0 / second_length);
        if cad_geometry::length(cross(unit, unit_second)) > MLEDIT_EPS {
            return Err(unsupported(
                "break_parallel",
                "MLEDIT BREAK removes one band across a multiline, so both selected lines must \
                 be parallel",
            ));
        }
        let s0 = dot(sub(band_start, first.start), unit);
        let s1 = dot(sub(band_end, first.start), unit);
        let (low, high) = if s0 <= s1 { (s0, s1) } else { (s1, s0) };
        if high - low <= MLEDIT_EPS {
            return Err(invalid(
                "break_range",
                "the two band corners project onto the same point; there is no band to remove",
            ));
        }
        let mut produced: Vec<ProducedEntity> = Vec::with_capacity(4);
        for line in [first, second] {
            // Both lines are parallel to `unit`, so their along-unit span is a
            // pure scalar projection in the first line's frame.
            let p_start = dot(sub(line.start, first.start), unit);
            let p_end = dot(sub(line.end, first.start), unit);
            let (from, to) = if p_start <= p_end {
                (p_start, p_end)
            } else {
                (p_end, p_start)
            };
            // The band must lie strictly inside the line; a band that misses it
            // or overhangs an end cannot be removed without inventing a segment.
            if low <= from + MLEDIT_EPS || high >= to - MLEDIT_EPS {
                return Err(invalid(
                    "break_range",
                    format!(
                        "the band [{low}, {high}] is not strictly inside entity {} (its extent \
                         is [{from}, {to}] on the shared direction)",
                        line.entity.id.0
                    ),
                ));
            }
            let cut_low = cad_geometry::add(line.start, scale(unit, low - p_start));
            let cut_high = cad_geometry::add(line.start, scale(unit, high - p_start));
            let (first_cut, second_cut) = if p_start <= p_end {
                (cut_low, cut_high)
            } else {
                // The stored direction runs against the shared direction.
                (cut_high, cut_low)
            };
            produced.push(ProducedEntity {
                geometry: SemanticGeometry::Line {
                    start: line.start,
                    end: first_cut,
                },
                layer: line.entity.layer,
                space: line.entity.space.clone(),
            });
            produced.push(ProducedEntity {
                geometry: SemanticGeometry::Line {
                    start: second_cut,
                    end: line.end,
                },
                layer: line.entity.layer,
                space: line.entity.space.clone(),
            });
        }
        let sources = vec![first.entity.clone(), second.entity.clone()];
        let mut outcome = replace_entities(
            self,
            command,
            sources,
            produced,
            "mledit break",
            "mledit.applied",
        )?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "mledit.residual".into(),
            message: "MLEDIT BREAK: the two-LINE subset removes the band from both parallel \
                      lines; MLINE caps, joins and fill are not modelled, so no cap or fill \
                      geometry is produced"
                .into(),
        });
        Ok(outcome)
    }

    /// Stage one `update_entity` per changed line in **one** transaction.
    fn commit_updates(
        &mut self,
        command: &Command,
        targets: Vec<(DbEntity, DbEntity)>,
        label: &str,
        diagnostics: Vec<Diagnostic>,
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
                CadError::Invariant("MLEDIT: an edited line vanished after commit".into())
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
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// The residual note every successful MLEDIT carries.
fn residual(mode: MlEditMode, intersection: Point3) -> Diagnostic {
    Diagnostic {
        object: None,
        code: "mledit.residual".into(),
        message: format!(
            "MLEDIT {}: edited two independent LINEs at ({}, {}); the stored model is the \
             two-LINE subset, so MLINE caps, miters, joins, fill and the AcDbMline vertex list \
             are not produced{}",
            mode.token(),
            intersection.x,
            intersection.y,
            if mode == MlEditMode::Open {
                " (the opened gap is zero-width because the model carries no multiline width)"
            } else {
                ""
            }
        ),
    }
}

/// Re-read one selected ref as a model-space `LINE`.
fn resolve_line(
    drawing: &cad_db::DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
) -> CadResult<SelectedLine> {
    if reference.document != command.document {
        return Err(invalid(
            "not_addressable",
            format!(
                "the ref for entity {} belongs to another document",
                reference.entity.0
            ),
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(unsupported(
            "not_addressable",
            format!(
                "entity {} is referenced through a block instance or a sub-element; pick the \
                 line entity itself",
                reference.entity.0
            ),
        ));
    }
    let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
        invalid(
            "target_missing",
            format!(
                "entity {} does not exist in this drawing",
                reference.entity.0
            ),
        )
    })?;
    if entity.space != SpaceId::Model {
        return Err(unsupported(
            "space",
            format!(
                "entity {} is not in model space; MLEDIT is model-space only",
                reference.entity.0
            ),
        ));
    }
    let (start, end) = match &entity.geometry {
        SemanticGeometry::Line { start, end } => (*start, *end),
        SemanticGeometry::Compound(_) if entity.object.type_key == "AcDbMline" => {
            return Err(unsupported(
                "acdbmline_model",
                format!(
                    "entity {} is a stored AcDbMline compound; this build keeps only its \
                     per-element polylines, so it has no editable join/cap/vertex model — use \
                     the two LINEs the MLINE command creates",
                    entity.id.0
                ),
            ))
        }
        other => {
            return Err(unsupported(
                "not_line",
                format!(
                    "entity {} is a {} entity, not a LINE",
                    entity.id.0,
                    class_name(other)
                ),
            ))
        }
    };
    let line = SelectedLine { entity, start, end };
    if line.length() <= MLEDIT_EPS {
        return Err(invalid(
            "degenerate",
            format!("entity {} is a zero-length LINE", line.entity.id.0),
        ));
    }
    Ok(line)
}

/// The intersection of two infinite lines, or a named refusal.
fn line_intersection(first: &SelectedLine, second: &SelectedLine) -> CadResult<Point3> {
    let r = first.direction();
    let s = second.direction();
    let denominator = cross(r, s).z;
    let scale_factor = first.length() * second.length();
    if !denominator.is_finite() || denominator.abs() <= MLEDIT_EPS * scale_factor {
        return Err(unsupported(
            "parallel",
            format!(
                "the selected lines are parallel or coincident (direction cross product \
                 {denominator}); they have no single intersection to clean up"
            ),
        ));
    }
    let delta = sub(second.start, first.start);
    let t = cross(delta, s).z / denominator;
    let intersection = cad_geometry::add(first.start, scale(r, t));
    if !is_finite_point(intersection) {
        return Err(invalid(
            "no_intersection",
            "the intersection of the two selected lines is not finite",
        ));
    }
    Ok(intersection)
}

/// Move the endpoint nearest `intersection` onto it; `None` when already there.
fn trim_or_extend(line: &SelectedLine, intersection: Point3) -> CadResult<Option<DbEntity>> {
    let distance_start = cad_geometry::distance(line.start, intersection);
    let distance_end = cad_geometry::distance(line.end, intersection);
    let epsilon = MLEDIT_EPS * (1.0 + line.length());
    let start_at_intersection = distance_start <= epsilon;
    let end_at_intersection = distance_end <= epsilon;
    if start_at_intersection && end_at_intersection {
        // Both ends coincide with the intersection: nothing to clean up.
        return Ok(None);
    }
    if start_at_intersection || end_at_intersection {
        // One end is already the intersection; the line meets it.
        return Ok(None);
    }
    let mut after = line.entity.clone();
    after.geometry = if distance_start <= distance_end {
        SemanticGeometry::Line {
            start: intersection,
            end: line.end,
        }
    } else {
        SemanticGeometry::Line {
            start: line.start,
            end: intersection,
        }
    };
    Ok(Some(after))
}

/// Whether `intersection` lies strictly inside the segment (not at an end).
fn strictly_inside(line: &SelectedLine, intersection: Point3) -> bool {
    let direction = line.direction();
    let length = line.length();
    let parameter = dot(sub(intersection, line.start), direction) / (length * length);
    let epsilon = MLEDIT_EPS;
    parameter > epsilon && parameter < 1.0 - epsilon
}
