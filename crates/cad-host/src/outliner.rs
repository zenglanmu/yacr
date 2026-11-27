//! The Outliner tray's read: the document hierarchy as a flat, bounded tree.
//!
//! The Outliner is a **projection of the document**, never a second model. The
//! authority stays in `cad-db`:
//!
//! * persisted groups (`tables/groups.json`) are top-level containers whose
//!   children are their member entities (the table guarantees direct refs);
//! * top-level model-space entities that are not group members are rows
//!   themselves; an `INSERT` row is a container whose children are the block
//!   definition's member entities;
//! * block members are **not** directly selectable through
//!   `cad_select_entities` (that wire addresses model-space ids), so a member row
//!   reports `selectable: false` and empty `selectIds` instead of a selection
//!   that would point at nothing.
//!
//! Rows are emitted in a pre-order walk with an explicit `depth`, so the panel
//! can expand/collapse without a second hierarchy request. `id` is the entity id
//! for entity rows or the group sequence for group rows; `selectIds` is exactly
//! what a click should send (a group sends all its members, one atomic request).
//!
//! Nothing here writes, tessellates or copies geometry. Rows are capped at
//! [`OUTLINER_MAX_ROWS`]; `total` always counts the whole projection and
//! `truncated` states that the page is short. No document is an explicit empty
//! answer (`revision: 0`), never an error.

use std::collections::{BTreeMap, HashSet};

use cad_domain::{EntityId, SemanticGeometry, SpaceId};

use crate::HostCore;

/// Most rows one Outliner answer carries.
///
/// A tree is a navigation surface, not a data export; a document with more rows
/// gets the first page plus `truncated: true` and its real `total`.
pub const OUTLINER_MAX_ROWS: usize = 1024;

/// One row of the Outliner tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlinerRow {
    /// `"group"`, `"insert"`, `"entity"` or `"member"`.
    pub kind: &'static str,
    /// Entity id for entity/insert/member rows, group sequence for group rows.
    pub id: u64,
    /// Display label (`LINE #12`, the block name, or the group name).
    pub label: String,
    /// The containing group's name for a group member row, else `None`.
    pub parent_group: Option<String>,
    /// Nesting depth (a pre-order walk; top-level rows are `0`).
    pub depth: u32,
    /// Whether this row has children in the same answer or beyond the page.
    pub has_children: bool,
    /// Whether the row is temporarily hidden in this session (`HIDEOBJECTS`).
    ///
    /// A group row is hidden when every member is hidden; a block member is
    /// never session-hidden on its own (its INSERT carries the hidden ref).
    pub hidden: bool,
    /// Whether a click can select this row through `cad_select_entities`.
    pub selectable: bool,
    /// The entity ids one click sends (empty when not selectable).
    pub select_ids: Vec<u64>,
}

/// The document hierarchy, bounded to the first [`OUTLINER_MAX_ROWS`] rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutlinerTree {
    /// The revision the rows were read from (`0` with no document).
    pub revision: u64,
    /// The first page of rows, in pre-order.
    pub rows: Vec<OutlinerRow>,
    /// The whole projection's row count (not the page length).
    pub total: usize,
    /// Whether `rows` is shorter than `total`.
    pub truncated: bool,
}

/// One entity's display label: its DXF type token plus the persistent id.
fn entity_label(type_key: &str, id: EntityId) -> String {
    format!("{type_key} #{}", id.0)
}

impl HostCore {
    /// The open document's Outliner tree, in pre-order.
    ///
    /// Runtime content only: the projection reads the current database and never
    /// mutates it, so calling it between edits is always safe.
    pub fn outliner_tree(&self) -> OutlinerTree {
        let controller = self.controller.borrow();
        let Some(database) = controller.drawing() else {
            return OutlinerTree::default();
        };
        let total_rows = count_rows(&database);
        let mut rows = Vec::with_capacity(total_rows.min(OUTLINER_MAX_ROWS));
        let mut push = |row: OutlinerRow| {
            if rows.len() < OUTLINER_MAX_ROWS {
                rows.push(row);
            }
        };
        let mut grouped: HashSet<EntityId> = HashSet::new();
        let document = controller.document_id;
        let hidden = &controller.session.hidden_entities;
        let is_hidden = |id: EntityId| {
            hidden.contains(&cad_domain::SelectionRef {
                document,
                entity: id,
                instance: cad_domain::InstancePath::default(),
                sub_element: None,
            })
        };
        for group in database.groups() {
            grouped.extend(group.members.iter().copied());
            let members: Vec<&cad_db::DbEntity> = group
                .members
                .iter()
                .filter_map(|id| database.entity(*id))
                .collect();
            let group_hidden =
                !members.is_empty() && members.iter().all(|entity| is_hidden(entity.id));
            push(OutlinerRow {
                kind: "group",
                id: group.seq.0,
                label: group.name.clone(),
                parent_group: None,
                depth: 0,
                hidden: group_hidden,
                has_children: !members.is_empty(),
                selectable: !members.is_empty(),
                select_ids: members.iter().map(|entity| entity.id.0 as u64).collect(),
            });
            for entity in members {
                push(OutlinerRow {
                    kind: "entity",
                    id: entity.id.0 as u64,
                    label: entity_label(&entity.object.type_key, entity.id),
                    parent_group: Some(group.name.clone()),
                    depth: 1,
                    hidden: is_hidden(entity.id),
                    has_children: false,
                    selectable: true,
                    select_ids: vec![entity.id.0 as u64],
                });
            }
        }
        // Top-level model-space entities in id order: stable output, and the same
        // id order the reference's "Model" list uses for ungrouped geometry.
        let mut top_level: Vec<&cad_db::DbEntity> = database
            .entities()
            .filter(|entity| entity.space == SpaceId::Model && !grouped.contains(&entity.id))
            .collect();
        top_level.sort_by_key(|entity| entity.id.0);
        for entity in top_level {
            match &entity.geometry {
                SemanticGeometry::Insert { block, .. } => {
                    let members: Vec<&cad_db::DbEntity> = database
                        .block(*block)
                        .map(|definition| {
                            definition
                                .entities
                                .iter()
                                .filter_map(|id| database.entity(*id))
                                .collect()
                        })
                        .unwrap_or_default();
                    push(OutlinerRow {
                        kind: "insert",
                        id: entity.id.0 as u64,
                        label: database
                            .block_name(*block)
                            .map(|name| format!("{name} #{}", entity.id.0))
                            .unwrap_or_else(|| entity_label(&entity.object.type_key, entity.id)),
                        parent_group: None,
                        depth: 0,
                        hidden: is_hidden(entity.id),
                        has_children: !members.is_empty(),
                        selectable: true,
                        select_ids: vec![entity.id.0 as u64],
                    });
                    for member in members {
                        push(OutlinerRow {
                            kind: "member",
                            id: member.id.0 as u64,
                            label: entity_label(&member.object.type_key, member.id),
                            parent_group: None,
                            depth: 1,
                            hidden: false,
                            has_children: false,
                            // A block member has no model-space address through
                            // `cad_select_entities`; the row is a readable leaf.
                            selectable: false,
                            select_ids: Vec::new(),
                        });
                    }
                }
                _ => push(OutlinerRow {
                    kind: "entity",
                    id: entity.id.0 as u64,
                    label: entity_label(&entity.object.type_key, entity.id),
                    parent_group: None,
                    depth: 0,
                    hidden: is_hidden(entity.id),
                    has_children: false,
                    selectable: true,
                    select_ids: vec![entity.id.0 as u64],
                }),
            }
        }
        OutlinerTree {
            revision: database.revision().0,
            total: total_rows,
            truncated: total_rows > rows.len(),
            rows,
        }
    }
}

/// The full row count the same walk would emit (used for `total`/`truncated`).
fn count_rows(database: &cad_db::DrawingDatabase) -> usize {
    let mut grouped: HashSet<EntityId> = HashSet::new();
    let mut count = 0usize;
    for group in database.groups() {
        grouped.extend(group.members.iter().copied());
        count += 1; // the group row
        count += group
            .members
            .iter()
            .filter(|id| database.entity(**id).is_some())
            .count();
    }
    let mut block_members: BTreeMap<u64, usize> = BTreeMap::new();
    for entity in database.entities() {
        if entity.space != SpaceId::Model || grouped.contains(&entity.id) {
            continue;
        }
        count += 1;
        if let SemanticGeometry::Insert { block, .. } = &entity.geometry {
            let members = block_members.entry(block.0 as u64).or_insert_with(|| {
                database
                    .block(*block)
                    .map(|definition| {
                        definition
                            .entities
                            .iter()
                            .filter(|id| database.entity(**id).is_some())
                            .count()
                    })
                    .unwrap_or(0)
            });
            count += *members;
        }
    }
    count
}
