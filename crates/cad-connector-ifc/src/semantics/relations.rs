//! Structural relations: IFC `IfcRel*` edges into [`cad_db::BimRelations`]
//! (ADR-0042 D3 tail / D7).
//!
//! The element pass stores each product's own BIM block and the shared
//! definition rows. This pass reads the *edges between* those objects — the
//! decomposition, nesting, spatial containment, grouping and connectivity the
//! file declares — into the bounded `tables/bim-relations.json` table.
//!
//! # Mapping
//!
//! | IFC record | `kind` | `parent` | `child` |
//! |---|---|---|---|
//! | `IfcRelAggregates` | `aggregates` | `RelatingObject` | each `RelatedObjects` |
//! | `IfcRelNests` | `nests` | `RelatingObject` | each `RelatedObjects` |
//! | `IfcRelContainedInSpatialStructure` | `contains` | `RelatingStructure` | each `RelatedElements` |
//! | `IfcRelAssignsToGroup` | `group` | `RelatingGroup` | each `RelatedObjects` |
//! | `IfcRelConnectsElements` | `connects` | `RelatingElement` | `RelatedElement` |
//!
//! Both endpoints are [`cad_db::BimKey::Source`] rows keyed by the object's own
//! `IfcRoot.GlobalId`; the project's `IfcProject.GlobalId` is the document id
//! (the same key the element `bim.source` block carries). The relationship
//! object's own `GlobalId`/`Name` become the optional `source`/`label`.
//!
//! # Losses and budgets
//!
//! A row whose parent or child has no readable `IfcRoot.GlobalId` is skipped
//! and counted under `import.ifc.bim.relation_endpoint_missing`; nothing is
//! invented from a step id. `(kind, parent, child)` duplicates are stored once
//! (the same edge re-declared across file partitions is normal). The row count
//! is bounded by [`cad_db::BIM_MAX_RELATIONS`]: a file that would exceed it
//! stops the scan and the whole table is **refused**, not truncated
//! (`import.ifc.bim.relation_budget_exceeded`). The encoded-size ceiling is
//! enforced by `cad_db` at `DrawingDatabaseBuilder::finish`, not estimated here.

use std::collections::{BTreeMap, BTreeSet};

use cad_db::{
    BimKey, BimRelation, BimRelationKind, BimRelations, BIM_MAX_RELATIONS, BIM_MAX_STRING_BYTES,
};
use ifc_lite_core::{slot, DecodedEntity, EntityDecoder, EntityScanner};

use super::CONNECTOR;

/// The outcome of one structural-relation scan.
#[derive(Default)]
pub(super) struct Scan {
    /// Collected edges. Empty when the scan was refused at the row budget.
    pub(super) relations: BimRelations,
    /// Edges skipped because an endpoint had no readable `IfcRoot.GlobalId`.
    pub(super) endpoint_missing: usize,
    /// The table exceeded [`BIM_MAX_RELATIONS`] and was refused (nothing stored).
    pub(super) budget_exceeded: bool,
}

/// Read the file's structural relations under the production row budget.
pub(super) fn collect(
    content: &[u8],
    decoder: &mut EntityDecoder<'_>,
    document_id: Option<&str>,
) -> Scan {
    collect_with_budget(content, decoder, document_id, BIM_MAX_RELATIONS)
}

/// [`collect`] with an explicit row budget, so a small synthetic file can prove
/// the refusal path without allocating `BIM_MAX_RELATIONS` rows.
fn collect_with_budget(
    content: &[u8],
    decoder: &mut EntityDecoder<'_>,
    document_id: Option<&str>,
    max_rows: usize,
) -> Scan {
    // A relation endpoint needs a non-empty document id. Without a readable
    // project GlobalId the `-` token is the same key component the definition
    // ids already use; it is a key component, not an invented document name.
    let document_id = document_id
        .filter(|id| !id.trim().is_empty() && id.len() <= BIM_MAX_STRING_BYTES)
        .unwrap_or("-");

    let mut scan = Scan::default();
    let mut seen: BTreeSet<(BimRelationKind, BimKey, BimKey)> = BTreeSet::new();
    let mut cache: BTreeMap<u32, Option<BimKey>> = BTreeMap::new();
    let mut scanner = EntityScanner::new(content);
    'scan: while let Some((id, type_name, start, end)) = scanner.next_entity() {
        if !is_wanted_relation(type_name) {
            continue;
        }
        let Ok(entity) = decoder.decode_at_with_id(id, start, end) else {
            // A relationship record that does not decode is a geometry-lane
            // style drop; there is no stable endpoint to report here.
            continue;
        };
        let Some(edge) = edge(&entity) else {
            continue;
        };
        let parent = endpoint_key(decoder, edge.parent, document_id, &mut cache);
        let source = key_from_entity(&entity, document_id);
        let label = entity
            .get_string(slot::IfcRoot::NAME.index())
            .map(str::trim)
            .filter(|label| !label.is_empty() && label.len() <= BIM_MAX_STRING_BYTES)
            .map(str::to_string);
        for child in edge.children {
            let child_key = endpoint_key(decoder, child, document_id, &mut cache);
            let (Some(parent_key), Some(child_key)) = (parent.as_ref(), child_key) else {
                // One endpoint has no GlobalId: the edge is not stored, and the
                // loss is counted rather than keyed by a step id.
                scan.endpoint_missing += 1;
                continue;
            };
            if !seen.insert((edge.kind.clone(), parent_key.clone(), child_key.clone())) {
                // `(kind, parent, child)` already stored: the source repeated it
                // across partitions. Keep the first row (deterministic).
                continue;
            }
            if scan.relations.len() >= max_rows {
                // Refuse the table rather than store a truncated prefix.
                scan.budget_exceeded = true;
                scan.relations = BimRelations::new();
                break 'scan;
            }
            let row = BimRelation {
                kind: edge.kind.clone(),
                parent: parent_key.clone(),
                child: child_key,
                source: source.clone(),
                label: label.clone(),
            };
            if scan.relations.insert(row).is_err() {
                // Only reachable for a key shape the checks above did not catch;
                // count it with the other skipped rows instead of dropping it
                // silently.
                scan.endpoint_missing += 1;
            }
        }
    }
    scan
}

/// One relation record's kind and raw step-id endpoints.
struct Edge {
    kind: BimRelationKind,
    parent: u32,
    children: Vec<u32>,
}

/// Decode one wanted relationship record into its typed edge.
///
/// A record whose required endpoint attributes are missing returns `None` and
/// is not counted here; the caller reports it as an endpoint loss only when a
/// child edge cannot be formed.
fn edge(entity: &DecodedEntity) -> Option<Edge> {
    match entity.ifc_type.as_str() {
        "IFCRELAGGREGATES" => {
            let parent = entity.get_ref(slot::IfcRelAggregates::RELATING_OBJECT.index())?;
            let children = entity.get_refs(slot::IfcRelAggregates::RELATED_OBJECTS.index())?;
            Some(Edge {
                kind: BimRelationKind::Aggregates,
                parent,
                children: children.to_vec(),
            })
        }
        "IFCRELNESTS" => {
            let parent = entity.get_ref(slot::IfcRelNests::RELATING_OBJECT.index())?;
            let children = entity.get_refs(slot::IfcRelNests::RELATED_OBJECTS.index())?;
            Some(Edge {
                kind: BimRelationKind::Nests,
                parent,
                children: children.to_vec(),
            })
        }
        "IFCRELCONTAINEDINSPATIALSTRUCTURE" => {
            let parent = entity
                .get_ref(slot::IfcRelContainedInSpatialStructure::RELATING_STRUCTURE.index())?;
            let children = entity
                .get_refs(slot::IfcRelContainedInSpatialStructure::RELATED_ELEMENTS.index())?;
            Some(Edge {
                kind: BimRelationKind::Contains,
                parent,
                children: children.to_vec(),
            })
        }
        "IFCRELASSIGNSTOGROUP" => {
            let parent = entity.get_ref(slot::IfcRelAssignsToGroup::RELATING_GROUP.index())?;
            let children = entity.get_refs(slot::IfcRelAssignsToGroup::RELATED_OBJECTS.index())?;
            Some(Edge {
                kind: BimRelationKind::Group,
                parent,
                children: children.to_vec(),
            })
        }
        "IFCRELCONNECTSELEMENTS" => {
            let parent = entity.get_ref(slot::IfcRelConnectsElements::RELATING_ELEMENT.index())?;
            let child = entity.get_ref(slot::IfcRelConnectsElements::RELATED_ELEMENT.index())?;
            Some(Edge {
                kind: BimRelationKind::Connects,
                parent,
                children: vec![child],
            })
        }
        _ => None,
    }
}

/// The one cheap gate before decoding: only these keywords carry structural edges.
fn is_wanted_relation(type_name: &str) -> bool {
    [
        "IFCRELAGGREGATES",
        "IFCRELNESTS",
        "IFCRELCONTAINEDINSPATIALSTRUCTURE",
        "IFCRELASSIGNSTOGROUP",
        "IFCRELCONNECTSELEMENTS",
    ]
    .iter()
    .any(|wanted| type_name.eq_ignore_ascii_case(wanted))
}

/// The `BimKey` of one referenced step id, with a per-scan decode cache.
fn endpoint_key(
    decoder: &mut EntityDecoder<'_>,
    step_id: u32,
    document_id: &str,
    cache: &mut BTreeMap<u32, Option<BimKey>>,
) -> Option<BimKey> {
    if let Some(cached) = cache.get(&step_id) {
        return cached.clone();
    }
    let key = decoder
        .decode_by_id(step_id)
        .ok()
        .and_then(|entity| key_from_entity(&entity, document_id));
    cache.insert(step_id, key.clone());
    key
}

/// A source key from an already-decoded root object, or `None` when it has no
/// usable `GlobalId`.
fn key_from_entity(entity: &DecodedEntity, document_id: &str) -> Option<BimKey> {
    let global_id = entity.get_string(slot::IfcRoot::GLOBAL_ID.index())?;
    if global_id.trim().is_empty() || global_id.len() > BIM_MAX_STRING_BYTES {
        return None;
    }
    Some(BimKey::Source {
        connector: CONNECTOR.to_string(),
        document_id: document_id.to_string(),
        id: global_id.to_string(),
    })
}

#[cfg(test)]
#[path = "../tests_semantics_relations.rs"]
mod tests_semantics_relations;
