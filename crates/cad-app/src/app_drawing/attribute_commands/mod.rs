//! Block-attribute commands (ADR-0055 M3): definitions, values, sync/redefine,
//! reorder, reports, display and `BURST`.
//!
//! Every write in this module goes through the database's one validated combined
//! path ([`cad_db::DrawingDatabase::apply_drawing_changes_with_blocks`]), so one
//! command is one transaction, one revision, one [`cad_db::ChangeSet`] and one
//! undo step — even when an `ATTREDEF` changes the definition *and* every
//! referencing `INSERT` in the same call. Every refusal happens before that call
//! and therefore writes nothing.
//!
//! The definition is a block **member entity** and a value is `INSERT` row data
//! (ADR-0055 D1), so "propagate a definition change" means explicitly rewriting
//! each referencing reference's value list; this module never relies on a
//! display cache to pick the new definition up.

use super::*;
use crate::command_payload::AttributeCommand;
use cad_db::{BlockPatch, BlockRow, ChangeScope, DbEntity, TableKind};
use cad_domain::{AttributeDefinition, AttributeValue, BlockId};
use cad_history::{drawing_patch, DrawingPatch, DrawingUndoRecord};

mod burst;
mod definition;
mod edit;
mod report;
mod sync;

/// A stable `attribute.<code>: <detail>` command refusal.
pub(super) fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("attribute.{code}: {detail}"))
}

/// A capability refusal that names the unsupported thing.
pub(super) fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("attribute.{code}: {detail}"))
}

/// Resolve a block name with the database's one rule (trimmed, exact then ASCII
/// case-insensitive).
pub(super) fn resolve_block(drawing: &DrawingDatabase, name: &str) -> CadResult<BlockId> {
    drawing
        .block_by_name(name)
        .map(|block| block.id)
        .ok_or_else(|| {
            refusal(
                "unknown_block",
                format!("no block named '{}' exists in this drawing", name.trim()),
            )
        })
}

/// The block's attribute definitions in their declared order.
///
/// The member list is the authority; equal `order` keys keep the member order
/// (stable sort), exactly like [`DrawingDatabase::default_attribute_values`].
/// A duplicate tag is reported here, because every definition write and every
/// tag-addressed value write must refuse it before touching a row.
pub(super) fn definitions(
    drawing: &DrawingDatabase,
    block: BlockId,
) -> CadResult<Vec<(EntityId, AttributeDefinition)>> {
    let definition = drawing
        .block(block)
        .ok_or_else(|| unsupported("unknown_block", format!("block {} is not stored", block.0)))?;
    let mut out = Vec::new();
    for member in &definition.entities {
        let Some(entity) = drawing.entity(*member) else {
            continue;
        };
        if let SemanticGeometry::AttributeDefinition(attribute) = &entity.geometry {
            out.push((entity.id, attribute.as_ref().clone()));
        }
    }
    out.sort_by_key(|(_, attribute)| attribute.order);
    let mut seen: Vec<&str> = Vec::new();
    for (_, attribute) in &out {
        if seen.iter().any(|tag| tags_equal(tag, &attribute.tag)) {
            return Err(refusal(
                "duplicate_tag",
                format!(
                    "block {} has more than one attribute definition tagged '{}'; tag-addressed \
                     attribute commands refuse an ambiguous definition",
                    block.0, attribute.tag
                ),
            ));
        }
        seen.push(attribute.tag.as_str());
    }
    Ok(out)
}

/// ASCII-case-insensitive tag identity (ADR-0055 D2).
pub(super) fn tags_equal(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Every stored `INSERT` of `block`, nested references included.
pub(super) fn references(drawing: &DrawingDatabase, block: BlockId) -> Vec<EntityId> {
    let mut out: Vec<EntityId> = drawing
        .entities()
        .filter(|entity| {
            matches!(
                &entity.geometry,
                SemanticGeometry::Insert {
                    block: referenced,
                    ..
                } if *referenced == block
            )
        })
        .map(|entity| entity.id)
        .collect();
    out.sort_by_key(|id| id.0);
    out
}

/// The attribute text a definition places a value at.
pub(super) fn text_from_definition(definition: &AttributeDefinition) -> AttributeText {
    definition.text.clone()
}

/// Materialize the value a fresh reference starts with.
pub(super) fn value_from_definition(
    definition: &AttributeDefinition,
    layer: LayerId,
) -> AttributeValue {
    AttributeValue::from_definition(definition, layer)
}

/// Reconcile one reference's stored values with the block's current definitions.
///
/// The rules are explicit (ADR-0055 D2/D3):
///
/// * an existing tag keeps its value and its own appearance/layer;
/// * a tag with no value is materialized from the definition's default;
/// * a `constant` definition's value is always the definition's default;
/// * the value's flags and text placement follow the definition, except that a
///   `locked_position` definition keeps the value's stored position;
/// * a value whose tag no longer matches a definition is dropped; the caller
///   reports the count instead of leaving a stale value behind.
pub(super) fn sync_values(
    definitions: &[(EntityId, AttributeDefinition)],
    layers: &[LayerId],
    old: &[AttributeValue],
) -> Vec<AttributeValue> {
    let mut out: Vec<AttributeValue> = Vec::with_capacity(definitions.len() + old.len());
    let mut used = vec![false; old.len()];
    for (position, (_, definition)) in definitions.iter().enumerate() {
        let layer_for_new = layers.get(position).copied().unwrap_or(LayerId(0));
        let matched = old
            .iter()
            .enumerate()
            .find(|(index, value)| !used[*index] && tags_equal(&value.tag, &definition.tag));
        let mut value = if let Some((index, previous)) = matched {
            used[index] = true;
            previous.clone()
        } else {
            value_from_definition(definition, layer_for_new)
        };
        value.tag = definition.tag.clone();
        value.flags = definition.flags;
        value.text = text_from_definition(definition);
        if definition.flags.locked_position {
            if let Some((index, previous)) = matched {
                debug_assert!(used[index]);
                value.text.position = previous.text.position;
            }
        }
        if definition.flags.constant {
            value.value = definition.default_value.clone();
        }
        out.push(value);
    }
    out
}

/// The patch list for an entity batch, read back from the committed candidate.
pub(super) fn entity_patches(
    before: &[(EntityId, Option<DbEntity>)],
    candidate: &DrawingDatabase,
) -> Vec<DrawingPatch> {
    before
        .iter()
        .map(|(id, old)| {
            let after = candidate.entity(*id).cloned();
            match old {
                Some(old) => drawing_patch(*id, Some(old.clone()), after),
                None => drawing_patch(*id, None, after),
            }
        })
        .collect()
}

/// Commit one attribute write through the combined block+entity path.
///
/// `before` is the pre-write row for every id in `changes` (`None` for an
/// insert); `blocks` carries the block-row creates/updates and their history
/// patches. On success the command returns the committed entity ids, the single
/// [`cad_db::ChangeSet`] and the caller's diagnostics.
#[allow(clippy::too_many_arguments)]
pub(super) fn commit_attribute_write(
    app: &mut Application,
    command: &Command,
    mut candidate: DrawingDatabase,
    label: &str,
    blocks: Vec<(BlockId, Option<BlockRow>)>,
    block_patches: Vec<BlockPatch>,
    changes: Vec<(EntityId, Option<DbEntity>)>,
    before: Vec<(EntityId, Option<DbEntity>)>,
    mut diagnostics: Vec<Diagnostic>,
) -> CadResult<CommandOutcome> {
    let transaction = next_transaction_id();
    let mut change_set =
        candidate.apply_drawing_changes_with_blocks(label, transaction, &blocks, changes)?;
    // ADR-0055 D3: a definition edit dirties the block table beside the block
    // model's element shards. The store re-encodes the table it holds, so this
    // is bookkeeping, not a raw-copy switch. A row-level definition edit
    // (ATTREDEF on an existing member, BATTORDER) has no block-row patch, so the
    // changed entity's space/kind decides it.
    let definition_edit =
        !block_patches.is_empty() || branch_edits_a_definition(&before, &candidate);
    if definition_edit {
        change_set.scope.table(TableKind::Blocks);
    }
    let objects: Vec<ObjectId> = before
        .iter()
        .filter_map(|(id, _)| candidate.entity(*id).map(|entity| ObjectId(entity.id.0)))
        .collect();
    let patches = entity_patches(&before, &candidate);
    app.publish_candidate(
        command.document,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.to_string(),
            patches,
            block_patches,
            layer_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    if diagnostics.is_empty() {
        diagnostics.push(Diagnostic {
            object: objects.first().copied(),
            code: "attribute.changed".into(),
            message: format!("{label}: wrote {} entity row(s)", change_set.changes.len()),
        });
    }
    Ok(CommandOutcome {
        objects,
        changes: Some(Arc::new(change_set)),
        diagnostics,
        measurement: None,
        point_report: None,
    })
}

/// A named no-op outcome: no revision, no history, no `ChangeSet`.
pub(super) fn unchanged(message: impl Into<String>) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "attribute.unchanged".into(),
            message: message.into(),
        }],
        measurement: None,
        point_report: None,
    }
}

/// Whether this batch changed a stored `ATTDEF` member (ADR-0055 D3 dirty scope).
fn branch_edits_a_definition(
    before: &[(EntityId, Option<DbEntity>)],
    candidate: &DrawingDatabase,
) -> bool {
    before.iter().any(|(id, _)| {
        candidate
            .entity(*id)
            .map(|entity| matches!(entity.geometry, SemanticGeometry::AttributeDefinition(_)))
            .unwrap_or(false)
    })
}

/// Read the open drawing clone or refuse.
pub(super) fn drawing_of(
    app: &Application,
    command: &Command,
) -> CadResult<(Arc<DrawingDatabase>, DrawingDatabase)> {
    let document = app
        .workspace
        .documents
        .get(&command.document)
        .ok_or_else(|| refusal("not_open", "no document is open"))?;
    Ok((
        Arc::clone(&document.drawing),
        document.drawing.as_ref().clone(),
    ))
}

/// A `ChangeScope` view of a committed edit (tests read it through the outcome).
#[allow(dead_code)]
pub(super) fn committed_scope(change_set: &cad_db::ChangeSet) -> &ChangeScope {
    &change_set.scope
}

impl Application {
    /// Dispatch one block-attribute command (ADR-0055 M3).
    ///
    /// The caller has already authorised the command and checked the document.
    /// `ATTDISP` stays with the Wave 5 session-token handler; the reports and
    /// writes here share the one payload/validator core.
    pub(crate) fn attribute_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::AttDef | ExtendedCommandId::DashAttDef => {
                if matches!(command.payload, CommandPayload::None) {
                    report::attdef_report(self, command)
                } else {
                    definition::attdef(self, session, command)
                }
            }
            ExtendedCommandId::AttRedef => definition::attredef(self, session, command),
            ExtendedCommandId::AttEdit => edit::edit_values(
                self,
                session,
                command,
                edit::EditTargets::OneSelected,
                "ATTEDIT",
            ),
            ExtendedCommandId::DashAttEdit => edit::edit_values(
                self,
                session,
                command,
                edit::EditTargets::AllInstances,
                "-ATTEDIT",
            ),
            ExtendedCommandId::EAttEdit => edit::edit_values(
                self,
                session,
                command,
                edit::EditTargets::Selected,
                "EATTEDIT",
            ),
            ExtendedCommandId::AttIpEdit => edit::edit_values(
                self,
                session,
                command,
                edit::EditTargets::OneSelected,
                "ATTIPEDIT",
            ),
            ExtendedCommandId::Gatte => edit::edit_values(
                self,
                session,
                command,
                edit::EditTargets::AllInstances,
                "GATTE",
            ),
            ExtendedCommandId::AttSync => sync::attsync(self, session, command),
            ExtendedCommandId::BattOrder => {
                if matches!(command.payload, CommandPayload::None) {
                    report::battorder_report(self, command)
                } else {
                    sync::battorder(self, command)
                }
            }
            ExtendedCommandId::BattMan => {
                if matches!(command.payload, CommandPayload::None) {
                    report::battman_report(self, command)
                } else {
                    Err(refusal("report_payload", "BATTMAN takes no payload"))
                }
            }
            ExtendedCommandId::Burst => burst::burst(self, session, command),
            other => Err(unsupported(
                "dispatch",
                format!("{} is not a block-attribute command", other.selector()),
            )),
        }
    }
}
