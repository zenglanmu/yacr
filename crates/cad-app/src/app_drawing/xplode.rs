//! `XPLODE` write path: EXPLODE's one-level block decomposition plus the
//! documented property/attribute controls.
//!
//! The decomposition is **not** reimplemented here: [`explode_source`] is the
//! same one-level walk `EXPLODE` and `BURST` use, including the placement
//! transform, the non-uniform conic subset and every named refusal. This module
//! only adds what XPLODE adds on top of it:
//!
//! * a resolved property list (`layer`/`color`/`linetype`/`lineweight`) applied
//!   to every produced child through the same [`plan_entity_with_attributes`]
//!   rule `SETENTITYPROPERTY`/`CHPROP` use;
//! * `Inherit from parent block`: a child whose block-definition layer is `0`
//!   moves to the exploded reference's layer, and a `ByBlock` colour / linetype
//!   / lineweight resolves against that reference (the AutoCAD rule for blocks
//!   built on layer `0` with `ByBlock` members);
//! * attribute values are **not** converted: the reference's stored values are
//!   dropped and the block definition's `ATTDEF` members are emitted as ordinary
//!   entities, which is the `EXPLODE`/`XPLODE` behaviour (`BURST` is the
//!   value-preserving command).
//!
//! Everything is committed in **one transaction / one undo step**: the sources
//! are deleted, the children inserted, and any display-attribute write staged in
//! the same transaction. A refusal happens during decomposition or property
//! resolution before the transaction opens, so it writes nothing.

use super::explode::{explode_source, next_explode_draw_order, Produced};
use super::properties::plan_entity_with_attributes;
use super::*;
use crate::property_value::PropertyValue;
use cad_db::{
    EntityColor, EntityLineType, EntityLineWeight, EntityRenderAttributes, EntityTransparency,
};
use cad_history::{drawing_patch, drawing_patch_deleted, DrawingPatch, DrawingUndoRecord};

/// The controls XPLODE adds to the shared EXPLODE decomposition.
pub(crate) struct XPlodeControls<'a> {
    /// Resolved property writes (`layer`/`color`/`linetype`/`lineweight`).
    /// Empty means the `Explode` option: the children keep the decomposition's
    /// ordinary result.
    pub(crate) targets: &'a [PropertyValue],
    /// `Inherit from parent block`: resolve layer `0` / `ByBlock` against the
    /// exploded reference. Mutually exclusive with `targets` (the command layer
    /// refuses the combination by name).
    pub(crate) inherit_from_parent: bool,
}

impl Application {
    /// XPLODE one or more block references: decompose one level, apply the
    /// selected property/attribute controls, and commit once.
    pub(crate) fn xplode_sources(
        &mut self,
        command: &Command,
        sources: Vec<DbEntity>,
        controls: &XPlodeControls<'_>,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        if sources.is_empty() {
            return Err(CadError::InvalidInput(
                "XPLODE needs a non-empty selection".into(),
            ));
        }

        // Capture the display state the delete will prune, so undo restores it.
        let display_states: Vec<_> = sources
            .iter()
            .map(|source| candidate.entity_display_state(source.id))
            .collect();

        // Decompose the whole selection before any write: XPLODE is
        // all-or-nothing, exactly like EXPLODE.
        let mut produced: Vec<Produced> = Vec::new();
        for source in &sources {
            explode_source(&candidate, source, true, &mut produced)?;
        }
        if produced.is_empty() {
            return Err(CadError::InvalidInput(
                "XPLODE produced no geometry from the selection".into(),
            ));
        }

        let refs: Vec<&PropertyValue> = controls.targets.iter().collect();
        let layer_zero = candidate.layer_zero_id();
        let base_order = next_explode_draw_order(&candidate);
        let mut children: Vec<(EntityId, DbEntity, Option<EntityRenderAttributes>)> =
            Vec::with_capacity(produced.len());
        for (index, child) in produced.into_iter().enumerate() {
            let id = candidate.allocate_entity_id();
            let mut entity = DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(id.0),
                    type_key: child.type_key,
                    revision: Revision(0),
                    source_handle: None,
                },
                id,
                layer: child.layer,
                space: child.space,
                geometry: child.geometry,
                draw_order: child.draw_order.unwrap_or(base_order + index as i64),
            };

            // The ordinary `Explode` result writes no display attributes, which
            // keeps XPLODE's default identical to EXPLODE. `Inherit` starts from
            // the member's own snapshot and resolves it against the reference.
            let mut display = EntityRenderAttributes::default();
            let mut write_display = false;
            if controls.inherit_from_parent {
                if let (Some(parent_layer), Some(zero)) = (child.parent_layer, layer_zero) {
                    if entity.layer == zero {
                        entity.layer = parent_layer;
                    }
                }
                if let Some(parent) = child.parent_display.as_ref() {
                    if let Some(member) = child.display.as_ref() {
                        display = member.clone();
                    }
                    inherit_attributes(&candidate, child.parent_layer, &mut display, parent);
                    write_display = true;
                }
            }

            // The typed controls go through the one property-write applicator.
            // A child has no committed row yet, so its display snapshot is the
            // one this transaction is about to write.
            if !controls.targets.is_empty() {
                if let Some(change) =
                    plan_entity_with_attributes(entity.clone(), display.clone(), &refs)
                {
                    entity = change.after;
                    if let Some(attributes) = change.after_attributes {
                        display = attributes;
                        write_display = true;
                    }
                }
            }

            children.push((id, entity, write_display.then_some(display)));
        }

        // One transaction: delete every source, insert every child, and stage
        // the property-derived display state alongside the rows.
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("xplode", transaction)?;
            for source in &sources {
                staged.delete_entity(source.id)?;
            }
            for (id, child, display) in &children {
                staged.insert_entity(child.clone())?;
                if let Some(attributes) = display {
                    staged.update_entity_display(*id, attributes.clone())?;
                }
            }
            staged.commit()?
        };

        let mut patches: Vec<DrawingPatch> = sources
            .iter()
            .zip(display_states)
            .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state))
            .collect();
        for (id, _, _) in &children {
            let after = candidate
                .entity(*id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("XPLODE: child vanished after commit".into()))?;
            let mut patch = drawing_patch(*id, None, Some(after));
            patch.after_state = Some(candidate.entity_display_state(*id));
            patches.push(patch);
        }

        let objects: Vec<ObjectId> = children.iter().map(|(id, _, _)| ObjectId(id.0)).collect();
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "xplode".into(),
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
            diagnostics: vec![Diagnostic {
                object: None,
                code: "xplode.decomposed".into(),
                message: format!(
                    "XPLODE: {} entities produced from {} block reference(s) (sources deleted)",
                    count,
                    sources.len()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve a member's `ByBlock` display values against the exploded reference.
///
/// An explicit reference value becomes a concrete child value. A reference that
/// is itself `ByLayer` is resolved through the reference layer's table row
/// (colour / lineweight / linetype / opacity), because `ByBlock` means "the
/// containing reference's effective value", not "my owning layer's value". A
/// reference that is itself `ByBlock` at model-space top level has no containing
/// reference; the child keeps `ByLayer` rather than inventing a concrete value.
fn inherit_attributes(
    drawing: &DrawingDatabase,
    parent_layer: Option<LayerId>,
    child: &mut EntityRenderAttributes,
    parent: &EntityRenderAttributes,
) {
    let layer = parent_layer
        .map(|id| drawing.layer_attributes(id))
        .unwrap_or_default();
    match child.color {
        EntityColor::ByBlock => {
            child.color = match parent.color {
                EntityColor::Explicit(rgb) => EntityColor::Explicit(rgb),
                EntityColor::ByLayer => EntityColor::Explicit(layer.color.rgb),
                EntityColor::ByBlock => EntityColor::ByLayer,
            };
        }
        EntityColor::Explicit(_) | EntityColor::ByLayer => {}
    }
    match child.lineweight {
        EntityLineWeight::ByBlock => {
            child.lineweight = match parent.lineweight {
                EntityLineWeight::Explicit(mm) => EntityLineWeight::Explicit(mm),
                EntityLineWeight::Default => EntityLineWeight::Default,
                EntityLineWeight::ByLayer => match layer.lineweight {
                    cad_db::LayerLineWeight::Hundredths(hundredths) => {
                        EntityLineWeight::Explicit(hundredths as f32 / 100.0)
                    }
                    cad_db::LayerLineWeight::Default => EntityLineWeight::Default,
                    cad_db::LayerLineWeight::ByLayer => EntityLineWeight::ByLayer,
                },
                EntityLineWeight::ByBlock => EntityLineWeight::ByLayer,
            };
        }
        EntityLineWeight::Explicit(_) | EntityLineWeight::Default | EntityLineWeight::ByLayer => {}
    }
    if child.linetype == EntityLineType::ByBlock {
        match &parent.linetype {
            EntityLineType::Explicit {
                name,
                pattern,
                scale,
            } => {
                child.linetype = EntityLineType::Explicit {
                    name: name.clone(),
                    pattern: pattern.clone(),
                    scale: *scale,
                };
                child.linetype_scale = *scale;
            }
            EntityLineType::ByLayer => {
                let resolved = drawing
                    .linetypes()
                    .find(|linetype| linetype.name == layer.linetype);
                child.linetype = match resolved {
                    Some(linetype) => EntityLineType::Explicit {
                        name: linetype.name.clone(),
                        pattern: linetype.pattern.clone(),
                        scale: parent.linetype_scale,
                    },
                    None => EntityLineType::ByLayer,
                };
                child.linetype_scale = parent.linetype_scale;
            }
            EntityLineType::ByBlock => {
                child.linetype = EntityLineType::ByLayer;
                child.linetype_scale = parent.linetype_scale;
            }
        }
    }
    if child.transparency == EntityTransparency::ByBlock {
        child.transparency = match parent.transparency {
            EntityTransparency::Explicit(opacity) => EntityTransparency::Explicit(opacity),
            EntityTransparency::ByLayer => EntityTransparency::Explicit(layer.opacity),
            EntityTransparency::ByBlock => EntityTransparency::ByLayer,
        };
    }
}
