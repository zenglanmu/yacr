//! Relation edges → export-plan placement (ADR-0042 D10 / D3 tail).
//!
//! `tables/bim-relations.json` is one bounded table of many-to-many edges. The
//! pinned plan API (`ifc-lite-export` 28.3.0) can express exactly two of the
//! stored families on a new object: one spatial container
//! (`IfcRelContainedInSpatialStructure`) and one nesting host (`IfcRelNests`).
//!
//! - `contains`: a child this export wrote is placed in the storey/building/space
//!   the edge names, overriding the level-derived default. A second edge with a
//!   different parent is a conversion loss, never a second relationship.
//! - `nests`: a child this export wrote is nested in that host. The dispositions
//!   are ordered host-first so the host already exists when the child is
//!   validated; a cycle cannot be written and one edge per cycle is dropped.
//! - `aggregates`, `group`, `connects`, `other:<token>`: no plan-level
//!   expression exists (nothing relates two products). Each edge is counted
//!   ([`crate::bim::RELATION_NOT_WRITTEN`]) and never faked as a container or a
//!   nest.
//!
//! An endpoint this export did not write is counted
//! ([`crate::bim::RELATION_ENDPOINT_MISSING`]) and the edge is skipped; no
//! endpoint is invented. Rows are visited in `(kind, parent, child)` order
//! (`BimRelations` is a `BTreeMap`), so the same document always resolves the
//! same way.

use std::collections::{BTreeMap, BTreeSet};

use cad_db::{BimKey, BimRelationKind, DrawingDatabase};
use ifc_lite_export::plan::{GlobalId, ObjectRef};

use crate::bim::Counts;

/// The exported objects a relation endpoint may name.
pub(crate) struct RelationInputs<'a> {
    /// Every mesh, by `BimKey` (source key and entity spelling).
    pub(crate) elements: &'a BTreeMap<BimKey, usize>,
    /// The GlobalId of each mesh, in mesh order.
    pub(crate) element_ids: &'a [GlobalId],
    /// The written storeys, by the key a relation names them with.
    pub(crate) storeys: &'a BTreeMap<BimKey, GlobalId>,
    /// The skeleton building (no source key of its own; see [`spatial_parent`]).
    pub(crate) building: GlobalId,
    /// The meshes written as a spatial element (`IfcSpace`).
    pub(crate) spatial: &'a BTreeSet<usize>,
    pub(crate) mesh_count: usize,
}

/// What the writable relation families place, resolved against this export.
#[derive(Default)]
pub(crate) struct RelationPlan {
    /// Explicit spatial container per mesh index (`contains`).
    pub(crate) contains: BTreeMap<usize, ObjectRef>,
    /// Nesting host per mesh index (`nests`).
    pub(crate) nests: BTreeMap<usize, usize>,
    /// The dispositions order that puts every nesting host before its
    /// children; the source order when nothing is nested.
    pub(crate) order: Vec<usize>,
}

/// Resolve the document's relation edges onto the objects this export writes.
///
/// `BimRelations` iterates in `(kind, parent, child)` order, so the outcome is
/// independent of the order the document stored the rows in. Only `contains`
/// and `nests` have a plan-level expression; every other kind is counted
/// ([`crate::bim::RELATION_NOT_WRITTEN`]) and never substituted for one of
/// those two.
pub(crate) fn plan_relations(
    database: &DrawingDatabase,
    inputs: &RelationInputs<'_>,
    counts: &mut Counts,
) -> RelationPlan {
    let mut plan = RelationPlan::default();
    for row in database.bim_relations().iter() {
        let kind = row.kind.as_token();
        match &row.kind {
            BimRelationKind::Contains => {
                let Some(&child) = inputs.elements.get(&row.child) else {
                    count_missing(counts, &kind);
                    continue;
                };
                match spatial_parent(&row.parent, inputs) {
                    None => {
                        // An exported object that is not a spatial element cannot
                        // be a container; one that is not exported at all is a
                        // missing endpoint.
                        if exported_object(&row.parent, inputs) {
                            count_unwritten(counts, &kind);
                        } else {
                            count_missing(counts, &kind);
                        }
                    }
                    Some(container) => match plan.contains.get(&child) {
                        // The plan API takes one container: the first edge in
                        // `(kind, parent, child)` order wins, the rest is a
                        // conversion loss, not a second relationship.
                        Some(chosen) if chosen != &container => {
                            count_unwritten(counts, &kind);
                        }
                        Some(_) => {}
                        None => {
                            plan.contains.insert(child, container);
                        }
                    },
                }
            }
            BimRelationKind::Nests => {
                let Some(&child) = inputs.elements.get(&row.child) else {
                    count_missing(counts, &kind);
                    continue;
                };
                // A spatial element is aggregated into its container by the
                // plan API; it cannot also be nested.
                if inputs.spatial.contains(&child) {
                    count_unwritten(counts, &kind);
                    continue;
                }
                let Some(&host) = inputs.elements.get(&row.parent) else {
                    // A storey or the building is written, but neither is a
                    // nesting host this writer can use.
                    if exported_object(&row.parent, inputs) {
                        count_unwritten(counts, &kind);
                    } else {
                        count_missing(counts, &kind);
                    }
                    continue;
                };
                if host == child || inputs.spatial.contains(&host) {
                    count_unwritten(counts, &kind);
                } else if let std::collections::btree_map::Entry::Vacant(slot) =
                    plan.nests.entry(child)
                {
                    // One nesting host per object: a second edge is a loss.
                    slot.insert(host);
                } else {
                    count_unwritten(counts, &kind);
                }
            }
            // `aggregates`, `group`, `connects` and every `other:<token>` edge
            // relate two products, which `ifc-lite-export` 28.3.0's plan has no
            // expression for. Faking one as a container/nest would state a
            // spatial fact the document did not declare, so each edge is
            // counted and the relation is left out.
            BimRelationKind::Aggregates
            | BimRelationKind::Group
            | BimRelationKind::Connects
            | BimRelationKind::Other(_) => count_unwritten(counts, &kind),
        }
    }

    plan.order = if plan.nests.is_empty() {
        (0..inputs.mesh_count).collect()
    } else {
        nest_order(&mut plan.nests, inputs.mesh_count, counts)
    };
    plan
}

/// The object a `contains` relation names, if this export wrote it as a space.
///
/// A storey resolves through its source key; the skeleton building has no
/// source key, so its own written `GlobalId` is the only honest match; an
/// exported `IfcSpace` may hold products too. Nothing else is a spatial
/// element, and nothing is invented.
fn spatial_parent(key: &BimKey, inputs: &RelationInputs<'_>) -> Option<ObjectRef> {
    if let Some(storey) = inputs.storeys.get(key) {
        return Some(ObjectRef::Guid(storey.encode()));
    }
    if let BimKey::Source { id, .. } = key {
        if id.as_str() == inputs.building.encode() {
            return Some(ObjectRef::Guid(inputs.building.encode()));
        }
    }
    inputs
        .elements
        .get(key)
        .filter(|index| inputs.spatial.contains(index))
        .map(|&index| ObjectRef::Guid(inputs.element_ids[index].encode()))
}

/// Whether a relation endpoint names any object this export wrote.
fn exported_object(key: &BimKey, inputs: &RelationInputs<'_>) -> bool {
    inputs.storeys.contains_key(key)
        || inputs.elements.contains_key(key)
        || matches!(key, BimKey::Source { id, .. } if id.as_str() == inputs.building.encode())
}

fn count_unwritten(counts: &mut Counts, kind: &str) {
    *counts
        .relation_not_written
        .entry(kind.to_string())
        .or_default() += 1;
}

fn count_missing(counts: &mut Counts, kind: &str) {
    *counts
        .relation_endpoint_missing
        .entry(kind.to_string())
        .or_default() += 1;
}

/// A disposition order in which every nesting host precedes its children.
///
/// `nests` maps child → host, so a cycle is a closed chain of hosts that no
/// order can satisfy; one edge per cycle is dropped and counted
/// ([`crate::bim::RELATION_NOT_WRITTEN`], kind `nests`). The walk is
/// deterministic: roots (objects with no host) first in index order, then the
/// remaining cycles in index order, each host's children in child order.
fn nest_order(
    nests: &mut BTreeMap<usize, usize>,
    mesh_count: usize,
    counts: &mut Counts,
) -> Vec<usize> {
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); mesh_count];
    for (&child, &host) in nests.iter() {
        if host < mesh_count && child < mesh_count {
            children[host].push(child);
        }
    }
    let mut state = vec![0u8; mesh_count];
    let mut order = Vec::with_capacity(mesh_count);

    // Roots are the objects no other object nests; walking from each root
    // reaches every acyclic child before it is written.
    for start in 0..mesh_count {
        if !nests.contains_key(&start) {
            nest_walk(start, &children, &mut state, &mut order, nests, counts);
        }
    }
    // An unvisited node then has a host but no path from any root: its host
    // chain is a cycle. Dropping one edge per such walk makes it a root.
    for start in 0..mesh_count {
        if state[start] == 0 {
            if nests.remove(&start).is_some() {
                count_unwritten(counts, "nests");
            }
            nest_walk(start, &children, &mut state, &mut order, nests, counts);
        }
    }
    order
}

/// Depth-first, host-first walk of the nesting tree from `start`.
fn nest_walk(
    start: usize,
    children: &[Vec<usize>],
    state: &mut [u8],
    order: &mut Vec<usize>,
    nests: &mut BTreeMap<usize, usize>,
    counts: &mut Counts,
) {
    if state[start] != 0 {
        return;
    }
    state[start] = 1;
    order.push(start);
    let mut stack = vec![(start, 0usize)];
    while let Some(&(node, position)) = stack.last() {
        let Some(&child) = children[node].get(position) else {
            state[node] = 2;
            stack.pop();
            continue;
        };
        if let Some(top) = stack.last_mut() {
            top.1 += 1;
        }
        match state[child] {
            0 => {
                state[child] = 1;
                order.push(child);
                stack.push((child, 0));
            }
            // Every node has at most one host, so an in-progress child can
            // only be this walk's own start: the edge into it closes the cycle.
            1 if nests.remove(&child).is_some() => count_unwritten(counts, "nests"),
            _ => {}
        }
    }
}
