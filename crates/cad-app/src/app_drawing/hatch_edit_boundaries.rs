//! `HATCHEDIT ADDBOUNDARIES` / `REMOVEBOUNDARIES` (AutoCAD 2026).
//!
//! Split out of `hatch_edit.rs` to stay inside the code-size gate. The
//! pre-selection model is the input: the session selection must hold exactly
//! **one** target hatch plus every other selected entity as a boundary source.
//! Each source is converted with the shared `HATCHSETBOUNDARY` projection
//! (`annot_batch::hatch::boundary_loop`), so a non-coplanar or unsupported shape
//! is refused by that code. The rebuilt loop set goes through
//! `HatchGeometry::new` (creation-path validation: duplicate/crossing rings are
//! named refusals) and the edit is one transaction / one undo step.

use super::annot_batch::boundary_loop;
use super::hatch_edit::{invalid, unsupported, Change, Target};
use super::*;
use crate::HatchEditOption;

impl Application {
    /// `HATCHEDIT ADDBOUNDARIES|REMOVEBOUNDARIES`: rewrite the one target
    /// hatch's loop set from the other selected closed entities.
    pub(super) fn hatch_edit_boundaries(
        &mut self,
        command: &Command,
        references: &[SelectionRef],
        before_drawing: &DrawingDatabase,
        candidate: DrawingDatabase,
        option: &HatchEditOption,
    ) -> CadResult<CommandOutcome> {
        let (target, sources) = resolve_boundary_edit(before_drawing, command, references)?;
        let mut source_loops = Vec::with_capacity(sources.len());
        for entity in &sources {
            source_loops.push(boundary_loop(entity, Some(&target.hatch))?);
        }
        let invalid_code = match option {
            HatchEditOption::AddBoundaries => "add_boundaries_invalid",
            HatchEditOption::RemoveBoundaries => "remove_boundaries_invalid",
            _ => unreachable!("only ADDBOUNDARIES/REMOVEBOUNDARIES reach here"),
        };
        let loops = match option {
            HatchEditOption::AddBoundaries => add_boundary_loops(&target.hatch, source_loops)?,
            HatchEditOption::RemoveBoundaries => {
                remove_boundary_loops(&target.hatch, &source_loops)?
            }
            _ => unreachable!("only ADDBOUNDARIES/REMOVEBOUNDARIES reach here"),
        };
        let rebuilt = HatchGeometry::new(
            loops,
            target.hatch.plane(),
            target.hatch.style(),
            target.hatch.fill().clone(),
            target.hatch.seeds().to_vec(),
            target.hatch.pixel_size(),
            target.hatch.pattern_type(),
        )
        .map_err(|error| {
            invalid(
                invalid_code,
                format!("the domain constructor rejected the new boundary set: {error}"),
            )
        })?;
        let mut after = target.entity.clone();
        after.geometry = SemanticGeometry::Hatch(Box::new(rebuilt));
        self.commit_hatch_edits(
            command,
            before_drawing,
            candidate,
            vec![Change {
                before: target.entity,
                after,
                display: None,
            }],
            Vec::new(),
            option,
        )
    }
}

/// Resolve the `ADDBOUNDARIES`/`REMOVEBOUNDARIES` selection: exactly one
/// directly addressable model-space hatch plus every other selected entity as a
/// boundary source (each is validated by `boundary_loop` when converted).
fn resolve_boundary_edit(
    drawing: &DrawingDatabase,
    command: &Command,
    references: &[SelectionRef],
) -> CadResult<(Target, Vec<DbEntity>)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut hatch: Option<Target> = None;
    let mut sources = Vec::new();
    for reference in references {
        if !seen.insert(reference.entity) {
            return Err(invalid(
                "duplicate",
                format!("entity {} is selected twice", reference.entity.0),
            ));
        }
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
                    "HATCHEDIT needs directly addressable model-space entities; entity {} is \
                     referenced through a block instance or a sub-element",
                    reference.entity.0
                ),
            ));
        }
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
                    "HATCHEDIT edits model-space hatches only; entity {} is in another space",
                    reference.entity.0
                ),
            ));
        }
        if let SemanticGeometry::Hatch(hatch_geometry) = &entity.geometry {
            if hatch.is_some() {
                return Err(invalid(
                    "multiple_hatches",
                    "ADDBOUNDARIES/REMOVEBOUNDARIES edit exactly one hatch, but more than one \
                     hatch is selected",
                ));
            }
            let hatch_geometry = hatch_geometry.as_ref().clone();
            hatch = Some(Target {
                entity,
                hatch: hatch_geometry,
            });
        } else {
            sources.push(entity);
        }
    }
    let Some(target) = hatch else {
        return Err(invalid(
            "selection",
            "ADDBOUNDARIES/REMOVEBOUNDARIES need exactly one selected hatch plus boundary \
             entities",
        ));
    };
    if sources.is_empty() {
        return Err(invalid(
            "no_boundary_source",
            "ADDBOUNDARIES/REMOVEBOUNDARIES need at least one selected closed boundary entity",
        ));
    }
    Ok((target, sources))
}

/// Append the converted boundary loops to the stored ones, refusing a set that
/// adds nothing (every source already is a stored loop).
fn add_boundary_loops(
    hatch: &HatchGeometry,
    incoming: Vec<HatchLoop>,
) -> CadResult<Vec<HatchLoop>> {
    let existing = hatch.loops();
    if incoming
        .iter()
        .all(|new| existing.iter().any(|stored| loops_match(stored, new)))
    {
        return Err(invalid(
            "add_boundaries_invalid",
            "every selected boundary already is a stored loop; ADDBOUNDARIES would change nothing",
        ));
    }
    let mut loops = existing.to_vec();
    loops.extend(incoming);
    Ok(loops)
}

/// Remove the stored loops matching a converted source by points + bulges.
fn remove_boundary_loops(
    hatch: &HatchGeometry,
    incoming: &[HatchLoop],
) -> CadResult<Vec<HatchLoop>> {
    let mut removed = 0usize;
    let mut remaining = Vec::new();
    for stored in hatch.loops() {
        if incoming.iter().any(|source| loops_match(stored, source)) {
            removed += 1;
        } else {
            remaining.push(stored.clone());
        }
    }
    if removed == 0 {
        return Err(invalid(
            "remove_no_match",
            "none of the selected boundaries matches a stored hatch loop (points + bulges, 1e-9 \
             tolerance); REMOVEBOUNDARIES would change nothing",
        ));
    }
    if remaining.is_empty() {
        return Err(invalid(
            "remove_last_loop",
            "removing the selected boundaries would leave the hatch with no boundary loop",
        ));
    }
    Ok(remaining)
}

/// Whether two loops are the same boundary for add/remove: same vertex count and
/// equal points + bulges within `1e-9` (flags and analytic source edges ignored).
fn loops_match(a: &HatchLoop, b: &HatchLoop) -> bool {
    const TOLERANCE: f64 = 1e-9;
    if a.points.len() != b.points.len() {
        return false;
    }
    a.points.iter().zip(&b.points).all(|(left, right)| {
        (left[0] - right[0]).abs() <= TOLERANCE && (left[1] - right[1]).abs() <= TOLERANCE
    }) && a
        .bulges
        .iter()
        .copied()
        .chain(std::iter::repeat(0.0))
        .zip(b.bulges.iter().copied().chain(std::iter::repeat(0.0)))
        .take(a.points.len())
        .all(|(left, right)| (left - right).abs() <= TOLERANCE)
}
