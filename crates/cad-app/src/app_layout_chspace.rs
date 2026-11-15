//! `CHSPACE`: move objects between model space and the active layout's paper
//! space through the active viewport's exact transform.
//!
//! `CHSPACE` used to be a Wave 4 named refusal
//! (`chspace.unsupported: moving entities between model and paper space is not
//! implemented`). It is now a real write: the selection is rewritten **once** —
//! every selected entity's geometry is mapped by the active viewport's resolved
//! transform (`to_paper` for model → paper, `to_model` for paper → model) and its
//! `space` is re-pointed, in one transaction and one undo step.
//!
//! # Direction
//!
//! The target space follows the selection, exactly like AutoCAD: entities in
//! model space go to the **active** paper layout, entities in the active paper
//! layout go to model space. A mixed selection is refused
//! (`chspace.mixed_space`) rather than split into two writes: one command is one
//! transaction, so a half-migration is not a state this command may produce. An
//! entity in another layout, a block-instance member and a sub-element are all
//! refused by name (`chspace.not_addressable`).
//!
//! # The transform
//!
//! The viewport is the session's active one
//! ([`crate::app_layout::paper_writes::target_viewport_index`]: the explicit
//! `VPMAX`-maximized viewport of this layout, else viewport `0`). Its transform
//! comes from [`cad_representation::viewport_transform`] — the same resolution
//! the display uses — so a viewport this build cannot frame exactly (a twisted,
//! mirrored, non-uniform, off-plane or `Partial` imported viewport) refuses the
//! whole command by name (`chspace.unsupported_viewport`) instead of moving
//! geometry by a guessed ratio. The mapped object keeps its *apparent* position
//! and size on the sheet, which is what makes the move visually stable.
//!
//! # Refusals (each writes nothing anywhere)
//!
//! * `chspace.needs_selection` — an empty selection.
//! * `chspace.paper_space_required` — no active paper layout (model space is not
//!   a layout, so there is no viewport to transform through).
//! * `chspace.no_viewport` — the active layout stores no viewport.
//! * `chspace.unsupported_viewport` — the active viewport cannot be resolved
//!   exactly by the representation layer.
//! * `chspace.not_addressable` — another document, another layout, a block
//!   instance member or a sub-element.
//! * `chspace.mixed_space` — the selection spans both spaces.
//! * `chspace.viewport_not_movable` — an `AcDbViewport` frame entity. A viewport
//!   is layout machinery, not an object; its space is owned by the layout row.
//! * `chspace.dimension_not_associative` — a `DIMENSION`. This build's dimensions
//!   carry no association model (ADR 0017), so the command cannot prove the
//!   dimension is not measuring objects that stay in the other space; refusing by
//!   name is honest, silently moving it would break the measurement.
//! * `chspace.untransformable` — a kind whose exact image this build cannot
//!   represent (a `MULTILEADER`/`SHAPE`/`OPAQUE`/`COMPOUND`, or a geometry whose
//!   mapped result fails validation). The reason names the entity and the kind.
//!
//! Viewer sessions are refused with [`CadError::PermissionDenied`] (this writes
//! the drawing).

use super::*;

use cad_db::{DbEntity, DrawingDatabase};
use cad_history::{drawing_patch, DrawingUndoRecord};
use cad_representation::{viewport_transform, ViewportState};

use crate::app_drawing::class_name;
use crate::app_layout::paper_writes::{require_work, target_viewport_index};
use crate::app_layout::{layout_viewports, require_paper_space};
use crate::app_utility::require_none_payload;

impl Application {
    /// `CHSPACE`: move the selection between model and paper space.
    pub(crate) fn ch_space(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "CHSPACE")?;
        require_work(session)?;
        let layout_id = require_paper_space(session, "chspace")?;
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let viewports = layout_viewports(&drawing, layout_id)?;
        let index = target_viewport_index(session, layout_id, None, &viewports, "chspace")?;
        // `Supported` carries both directions; a viewport the representation
        // layer cannot frame exactly is refused, never approximated.
        let (to_paper, to_model, paper_per_model) = match viewport_transform(&viewports[index]) {
            ViewportState::Supported(transform) => (
                transform.to_paper,
                transform.to_model,
                transform.paper_per_model,
            ),
            ViewportState::Unsupported(reason) => {
                return Err(CadError::Unsupported(format!(
                    "chspace.unsupported_viewport: viewport {index} of layout {} cannot be \
                     resolved exactly, so CHSPACE cannot transform through it: {reason}",
                    layout_id.0
                )))
            }
        };
        let sources = addressable_selection(&drawing, session, command, layout_id)?;
        // One direction per command: a mixed selection cannot be one transaction
        // that means two different things.
        let to_paper_direction =
            match sources.first().map(|entity| &entity.space) {
                Some(SpaceId::Model) => true,
                Some(SpaceId::Paper(id)) if *id == layout_id => false,
                _ => return Err(CadError::InvalidInput(
                    "chspace.needs_selection: CHSPACE needs at least one selected entity in model \
                     space or in the active paper layout"
                        .into(),
                )),
            };
        for entity in &sources {
            let same_direction = match (&entity.space, to_paper_direction) {
                (SpaceId::Model, true) => true,
                (SpaceId::Paper(id), false) => *id == layout_id,
                _ => false,
            };
            if !same_direction {
                return Err(CadError::InvalidInput(format!(
                    "chspace.mixed_space: entity {} is in {:?} but the first selected entity is in \
                     the other space; CHSPACE moves one selection in one direction per transaction",
                    entity.id.0, entity.space
                )));
            }
        }
        let target_space = if to_paper_direction {
            SpaceId::Paper(layout_id)
        } else {
            SpaceId::Model
        };
        let transform = if to_paper_direction {
            to_paper
        } else {
            to_model
        };
        let mut planned = Vec::with_capacity(sources.len());
        for before in sources {
            if before.object.type_key == "AcDbViewport" {
                return Err(CadError::Unsupported(format!(
                    "chspace.viewport_not_movable: entity {} is an AcDbViewport frame; a viewport \
                     belongs to its layout row, not to the entity table, so CHSPACE does not move \
                     it",
                    before.id.0
                )));
            }
            if matches!(before.geometry, SemanticGeometry::Dimension(_)) {
                return Err(CadError::Unsupported(format!(
                    "chspace.dimension_not_associative: entity {} is a DIMENSION and this build's \
                     dimensions carry no association model, so CHSPACE cannot prove it does not \
                     measure objects that stay in the other space",
                    before.id.0
                )));
            }
            let geometry =
                cad_db::transform_geometry(&before.geometry, &transform).map_err(|error| {
                    CadError::Unsupported(format!(
                        "chspace.untransformable: entity {} ({}) cannot be moved to {:?} exactly: \
                         {}",
                        before.id.0,
                        class_name(&before.geometry),
                        target_space,
                        error_detail(error)
                    ))
                })?;
            let mut after = before.clone();
            after.geometry = geometry;
            after.space = target_space.clone();
            planned.push((before, after));
        }
        let count = planned.len();
        let (objects, change_set) =
            commit_space_changes(self, document_id, drawing, &planned, "chspace")?;
        let factor = if to_paper_direction {
            format!("model → paper at {paper_per_model} paper units per model unit")
        } else {
            format!(
                "paper → model at {} model units per paper unit",
                1.0 / paper_per_model
            )
        };
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "chspace.moved".into(),
                message: format!(
                    "CHSPACE: moved {count} entity/entities {factor} through viewport {index} of \
                     layout {} into {target_space:?}; one transaction / one undo step",
                    layout_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve the selection to directly addressable entities of model space or
/// `layout_id`.
///
/// Fails by name for an empty selection, another document, a block-instance
/// member, a sub-element, a missing entity and an entity in another layout. The
/// direction itself is decided by the caller (see [`Application::ch_space`]).
fn addressable_selection(
    drawing: &DrawingDatabase,
    session: &SessionState,
    command: &Command,
    layout_id: LayoutId,
) -> CadResult<Vec<DbEntity>> {
    let references = session.selection.refs().to_vec();
    if references.is_empty() {
        return Err(CadError::InvalidInput(
            "chspace.needs_selection: CHSPACE needs at least one selected entity".into(),
        ));
    }
    let mut entities = Vec::with_capacity(references.len());
    let mut seen = std::collections::BTreeSet::new();
    for reference in &references {
        if reference.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "chspace.not_addressable: target {} belongs to another document",
                reference.entity.0
            )));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(CadError::Unsupported(format!(
                "chspace.not_addressable: target {} is a block-instance member or sub-element; \
                 only directly addressable entities are supported",
                reference.entity.0
            )));
        }
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            CadError::InvalidInput(format!(
                "chspace.target_missing: target {} does not exist",
                reference.entity.0
            ))
        })?;
        match &entity.space {
            SpaceId::Model => {}
            SpaceId::Paper(id) if *id == layout_id => {}
            other => {
                return Err(CadError::Unsupported(format!(
                    "chspace.not_addressable: target {} is in {other:?}, which is neither model \
                     space nor the active layout {}",
                    entity.id.0, layout_id.0
                )))
            }
        }
        if seen.insert(entity.id) {
            entities.push(entity);
        }
    }
    Ok(entities)
}

/// Commit every planned `(before, after)` rewrite as one transaction / one undo
/// step.
///
/// The space re-point goes through the combined validated write (the same entry
/// `LAYOUTMERGE` uses), so block membership, render attributes and the layer-lock
/// chokepoint are enforced by the database rather than by this command.
fn commit_space_changes(
    app: &mut Application,
    document_id: DocumentId,
    mut candidate: DrawingDatabase,
    planned: &[(DbEntity, DbEntity)],
    label: &str,
) -> CadResult<(Vec<ObjectId>, Arc<ChangeSet>)> {
    let transaction = next_transaction_id();
    let changes: Vec<(EntityId, Option<DbEntity>)> = planned
        .iter()
        .map(|(_, after)| (after.id, Some(after.clone())))
        .collect();
    let change_set = candidate.apply_drawing_changes_with_blocks_display_layers_and_layouts(
        label,
        transaction,
        &[],
        changes,
        &[],
        &[],
        &[],
    )?;
    let mut patches = Vec::with_capacity(planned.len());
    let mut objects = Vec::with_capacity(planned.len());
    for (before, _) in planned {
        let after = candidate
            .entity(before.id)
            .cloned()
            .ok_or_else(|| CadError::Invariant(format!("{label}: an updated entity vanished")))?;
        patches.push(drawing_patch(before.id, Some(before.clone()), Some(after)));
        objects.push(ObjectId(before.id.0));
    }
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.into(),
            patches,
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok((objects, Arc::new(change_set)))
}

/// The message of one refusal, without its `CadError` wrapper.
fn error_detail(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => format!("{other:?}"),
    }
}
