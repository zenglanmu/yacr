//! Shared-cell definitions (DGN blocks) and their member entities.

use std::collections::{BTreeMap, BTreeSet};

use cad_db::BlockDefinition;
use cad_domain::{BlockId, CadError, CadResult, EntityId, SpaceId};
use cadkit_core::{Document, Entity, EntityKind};

use super::{Inherit, Mapper};
use crate::DEPTH_LIMIT;

impl Mapper<'_> {
    /// Map every shared cell definition's members into its block space.
    pub(crate) fn map_blocks(&mut self) -> CadResult<()> {
        let doc = self.doc;
        for block in &doc.blocks {
            if (self.cancelled)() {
                return Err(CadError::Cancelled);
            }
            let Some(id) = self.blocks.get(&block.name).copied() else {
                continue;
            };
            // A duplicated name resolves to its last definition; only that one
            // owns the members, so the earlier row cannot be overwritten with a
            // different member list.
            if self.active_blocks.contains(&id) {
                continue;
            }
            self.active_blocks.insert(id);
            let members =
                self.map_entities(&block.entities, &SpaceId::Block(id), &Inherit::default(), 0)?;
            self.active_blocks.remove(&id);
            self.builder.insert_block(BlockDefinition {
                id,
                entities: members,
                dynamic_visibility: None,
            })?;
        }
        Ok(())
    }

    /// Map the primary design model and report every skipped model.
    pub(crate) fn map_primary_model(&mut self) -> CadResult<()> {
        let Some(primary) = self.primary_model_index() else {
            return Err(CadError::Unsupported(
                crate::NO_GEOMETRY.to_string() + ": the DGN file declares no model",
            ));
        };
        let doc = self.doc;
        for (index, model) in doc.models.iter().enumerate() {
            if index == primary {
                continue;
            }
            self.skipped_models += 1;
            self.note(
                crate::MODEL_SKIPPED,
                format!(
                    "model '{}' ({:?}) is not the primary design model; this build imports only \
                     one model, so its {} top-level entity row(s) were not read",
                    model.name,
                    model.kind,
                    model.entities.len()
                ),
                None,
            );
        }
        let model = doc
            .models
            .get(primary)
            .ok_or_else(|| super::invariant("primary DGN model index is out of bounds"))?;
        self.map_entities(&model.entities, &SpaceId::Model, &Inherit::default(), 0)?;
        Ok(())
    }

    pub(super) fn map_entities(
        &mut self,
        entities: &[Entity],
        space: &SpaceId,
        inherit: &Inherit,
        depth: usize,
    ) -> CadResult<Vec<EntityId>> {
        let mut ids = Vec::new();
        for entity in entities {
            if (self.cancelled)() {
                return Err(CadError::Cancelled);
            }
            if depth > self.max_depth {
                self.depth_limited += 1;
                continue;
            }
            self.tags += entity.attributes.len();
            match &entity.kind {
                EntityKind::Group { children, .. } => {
                    self.groups_flattened += 1;
                    let style = self.resolve_style(entity, inherit)?;
                    let mut child_ids =
                        self.map_entities(children, space, &style.inherited(), depth + 1)?;
                    ids.append(&mut child_ids);
                }
                _ => {
                    if let Some(id) = self.map_one(entity, space, inherit)? {
                        ids.push(id);
                    }
                }
            }
        }
        Ok(ids)
    }

    /// Whether instantiating `block` would recurse: it is either the definition
    /// currently being mapped, or its insert chain closes a cycle / exceeds the
    /// import depth limit. The instance is skipped and reported, never expanded.
    pub(super) fn is_active_block(&self, block: BlockId) -> bool {
        self.active_blocks.contains(&block) || self.block_unsafe.contains(&block)
    }

    pub(super) fn note_depth_limit(&mut self) {
        self.depth_limited += 1;
        self.note(
            DEPTH_LIMIT,
            "a shared cell definition references itself, closes a cycle, or nests deeper than the \
             import limit; the instance was skipped rather than emitted as a recursive block \
             reference"
                .into(),
            None,
        );
    }
}

/// Collect every block name an entity tree references through `Insert`.
pub(super) fn insert_targets<'b>(entities: &'b [Entity], out: &mut Vec<&'b str>) {
    for entity in entities {
        match &entity.kind {
            EntityKind::Insert { block, .. } => out.push(block.as_str()),
            EntityKind::Group { children, .. } => insert_targets(children, out),
            _ => {}
        }
    }
}

/// Longest insert chain starting at `name` (0 for a leaf definition).
pub(super) fn block_depth<'b>(
    doc: &'b Document,
    name: &'b str,
    visiting: &mut BTreeSet<String>,
    memo: &mut BTreeMap<String, usize>,
    cycles: &mut usize,
    cyclic: &mut BTreeSet<String>,
) -> usize {
    if let Some(depth) = memo.get(name) {
        return *depth;
    }
    if !visiting.insert(name.to_string()) {
        // `A -> B -> A`: the chain is a cycle, not a finite nesting depth.
        *cycles += 1;
        cyclic.insert(name.to_string());
        return 0;
    }
    let mut depth = 0;
    if let Some(block) = doc.blocks.iter().find(|block| block.name == name) {
        let mut targets = Vec::new();
        insert_targets(&block.entities, &mut targets);
        for target in targets {
            if target == name {
                depth = depth.max(1);
                continue;
            }
            depth = depth.max(1 + block_depth(doc, target, visiting, memo, cycles, cyclic));
        }
    }
    visiting.remove(name);
    memo.insert(name.to_string(), depth);
    depth
}
