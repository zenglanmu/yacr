//! `SETBYLAYER`: reset the modelled display overrides (colour, linetype,
//! lineweight, transparency) to symbolic `ByLayer`, optionally recursing into
//! selected `INSERT`s.
//!
//! Split out of `layer_commands` so both files stay inside the code-size gate;
//! the declared boundaries (what is *not* reset, the lock refusal) are
//! documented on the helpers and the method below.

use crate::{SelectionToolPayload, SetByLayerSettings};
use cad_db::{
    EntityColor, EntityLineType, EntityLineWeight, EntityRenderAttributes, EntityTransparency,
};

use super::*;

/// The `SETBYLAYER` informational diagnostic: what AutoCAD's command also resets
/// but this build deliberately does not.
fn properties_not_reset(include_blocks: bool) -> Diagnostic {
    let include = if include_blocks {
        "Include blocks was requested and block members were included"
    } else {
        "Include blocks was not requested"
    };
    Diagnostic {
        object: None,
        code: "setbylayer.properties_not_reset".into(),
        message: format!(
            "SETBYLAYER: materials are not reset, and text style/height, dimension style, hatch \
             colour/pattern and polyline thickness are not modelled; {include}"
        ),
    }
}

/// Read the optional `SETBYLAYER` options; `None` means the bare command.
fn setbylayer_settings(command: &Command) -> CadResult<SetByLayerSettings> {
    match &command.payload {
        CommandPayload::None => Ok(SetByLayerSettings::default()),
        CommandPayload::SelectionTool(payload) => match payload.as_ref() {
            SelectionToolPayload::SetByLayer(settings) => Ok(*settings),
            _ => Err(invalid(
                "setbylayer",
                "payload",
                "SETBYLAYER accepts only its own typed settings",
            )),
        },
        _ => Err(invalid(
            "setbylayer",
            "payload",
            "SETBYLAYER takes no payload or its typed settings",
        )),
    }
}

/// Collect the block members of `block`, recursing into nested `INSERT`s.
///
/// Member entities live in [`SpaceId::Block`]; only their display state changes,
/// so the authoritative block row/member list is not touched. `visited` prevents
/// a cyclic block graph from looping, and `targets` is the deduplicated list of
/// entity ids already scheduled (a block referenced twice is written once).
fn collect_block_members(
    drawing: &DrawingDatabase,
    block: BlockId,
    visited: &mut Vec<BlockId>,
    targets: &mut Vec<EntityId>,
) -> CadResult<()> {
    if visited.contains(&block) {
        return Ok(());
    }
    visited.push(block);
    let definition = drawing.block(block).ok_or_else(|| {
        unsupported(
            "setbylayer",
            "include_blocks_missing_block",
            format!("block {} does not exist in this drawing", block.0),
        )
    })?;
    for member in definition.entities.clone() {
        let entity = drawing.entity(member).ok_or_else(|| {
            unsupported(
                "setbylayer",
                "include_blocks_missing_member",
                format!("block {} lists missing member entity {}", block.0, member.0),
            )
        })?;
        if !targets.contains(&member) {
            targets.push(member);
        }
        if let SemanticGeometry::Insert { block: nested, .. } = &entity.geometry {
            collect_block_members(drawing, *nested, visited, targets)?;
        }
    }
    Ok(())
}

impl Application {
    /// `SETBYLAYER`: reset the modelled display overrides to ByLayer.
    pub(super) fn setbylayer(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let settings = setbylayer_settings(command)?;
        let references = selection_snapshot(session, "setbylayer", "SETBYLAYER")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        for reference in &references {
            ensure_model_space_addressable(drawing.as_ref(), command, reference, "setbylayer")?;
        }

        // The write set: the selected rows, plus (with Include blocks) every
        // member of every selected INSERT, recursively.
        let mut targets: Vec<EntityId> = references
            .iter()
            .map(|reference| reference.entity)
            .collect();
        let mut block_member_count = 0usize;
        if settings.include_blocks {
            let mut visited: Vec<BlockId> = Vec::new();
            for reference in &references {
                let entity = drawing
                    .entity(reference.entity)
                    .expect("preflight verified the entity resolves");
                if let SemanticGeometry::Insert { block, .. } = &entity.geometry {
                    let before = targets.len();
                    collect_block_members(drawing.as_ref(), *block, &mut visited, &mut targets)?;
                    block_member_count += targets.len() - before;
                }
            }
        }

        // AutoCAD does not modify objects on locked layers. The display-write
        // path deliberately bypasses the lock (it is a representation rebuild),
        // so this command owns the refusal for the whole batch — selected rows
        // and included block members alike.
        for id in &targets {
            let entity = drawing
                .entity(*id)
                .ok_or_else(|| CadError::Invariant("SETBYLAYER target vanished".into()))?;
            if drawing.layer_locked(entity.layer) {
                let name = drawing
                    .layer(entity.layer)
                    .map(|row| row.name.as_str())
                    .unwrap_or("<unknown>");
                return Err(invalid(
                    "setbylayer",
                    "locked",
                    format!(
                        "entity {} is on locked layer '{name}'; AutoCAD does not modify objects \
                         on locked layers, so the whole command is refused",
                        entity.id.0
                    ),
                ));
            }
        }

        // Only the four modelled overrides change. Materials,
        // text/dimension/hatch/polyline properties are outside this reset (see
        // `properties_not_reset`), and `linetype_scale` is preserved because the
        // symbolic `ByLayer` variant carries no scale of its own.
        let mut changes: Vec<(EntityId, DbEntity, EntityRenderAttributes)> = Vec::new();
        for id in &targets {
            let before = drawing
                .entity(*id)
                .ok_or_else(|| CadError::Invariant("SETBYLAYER target vanished".into()))?
                .clone();
            let mut attributes = drawing.entity_render_attributes(before.id);
            if attributes.color == EntityColor::ByLayer
                && attributes.linetype == EntityLineType::ByLayer
                && attributes.lineweight == EntityLineWeight::ByLayer
                && attributes.transparency == EntityTransparency::ByLayer
            {
                continue;
            }
            attributes.color = EntityColor::ByLayer;
            attributes.linetype = EntityLineType::ByLayer;
            attributes.lineweight = EntityLineWeight::ByLayer;
            attributes.transparency = EntityTransparency::ByLayer;
            changes.push((before.id, before, attributes));
        }

        if changes.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![
                    Diagnostic {
                        object: None,
                        code: "setbylayer.unchanged".into(),
                        message: "SETBYLAYER: colour, linetype, lineweight and transparency are \
                                  already ByLayer on every target entity"
                            .into(),
                    },
                    properties_not_reset(settings.include_blocks),
                ],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("setbylayer", transaction)?;
            for (id, _, attributes) in &changes {
                staged.update_entity_display(*id, attributes.clone())?;
            }
            staged.commit()?
        };

        let mut patches = Vec::with_capacity(changes.len());
        let mut objects = Vec::with_capacity(changes.len());
        for (id, before, _) in &changes {
            patches.push(DrawingPatch {
                id: *id,
                before: Some(before.clone()),
                after: Some(before.clone()),
                before_state: Some(drawing.entity_display_state(*id)),
                after_state: Some(candidate.entity_display_state(*id)),
            });
            objects.push(ObjectId(id.0));
        }
        let count = changes.len();
        let block_note = if settings.include_blocks {
            format!("; {block_member_count} block member(s) included")
        } else {
            String::new()
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "setbylayer".to_string(),
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

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "setbylayer.applied".into(),
                    message: format!(
                        "SETBYLAYER: reset colour, linetype, lineweight and transparency to \
                         ByLayer on {count} entities (one transaction / one undo step){block_note}"
                    ),
                },
                properties_not_reset(settings.include_blocks),
            ],
            measurement: None,
            point_report: None,
        })
    }
}
