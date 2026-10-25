//! LINE/ARC boundary stitching for HATCH (ADR 0018 decision D, extension).
//!
//! The HATCH selection snapshot may contain `LINE` and `ARC` entities. One such
//! entity is **not** a loop, so the command chains the selected edges into closed
//! loops here and hands each loop to the rest of the module as a closed bulged
//! polyline: every `ARC` edge becomes one **exact** bulge segment
//! (`bulge = tan(sweep / 4)`, the same convention the circle loop uses) and every
//! `LINE` edge a bulge-0 segment. Arcs are never sampled — the ellipse path is the
//! only sampled boundary in this command.
//!
//! The algorithm is deliberately small and predictable:
//!
//! 1. **Node clustering.** Each edge endpoint is assigned to a node: the first
//!    endpoint within [`STITCH_TOLERANCE`] becomes that node's position and later
//!    endpoints are moved onto it. The tolerance is **absolute** (world units)
//!    because the selection comes from model space in world coordinates and this
//!    layer has no drawing scale, no unit context and no view scale to derive a
//!    relative epsilon from; inventing one would silently accept larger gaps in
//!    large drawings. `1e-6` world units is the DXF/`WCS` chaining magnitude: a
//!    genuine endpoint match is exact (or differs by double-rounding noise, many
//!    orders below it), while a visible gap is refused. When a gap exceeds the
//!    tolerance the nodes stay distinct, the chain ends there and the loop set is
//!    refused as open — nothing is ever bridged by extrapolation.
//! 2. **Branch check.** A node where more than two edge ends meet has no unique
//!    pairing, so the whole command refuses it (`hatch.boundary_branch`, naming
//!    the node and how many edges meet there). A pairing is never guessed.
//! 3. **Chain walk.** From an unused edge, walk out of each node through its only
//!    other unused edge. A walk that returns to the node it started at is one
//!    closed loop; a walk that dead-ends is an open chain. Edges are **undirected
//!    for matching but directed for geometry**: an edge may be consumed in either
//!    direction (`start→end` or `end→start`), so a selection whose LINEs/ARCs were
//!    drawn in mixed orientations still stitches. The traversal direction decides
//!    the segment's bulge sign (`tan(+sweep/4)` forward, `tan(-sweep/4)` when the
//!    arc is consumed backwards), which is what keeps the arcs exact.
//! 4. **Coverage.** Every selected edge must be consumed by exactly one loop.
//!    Leftover edges are an open chain and are refused by name
//!    (`hatch.boundary_open`) together with the number of unconsumed edges.
//!
//! Degenerate input (a zero-length `LINE`, a zero-sweep `ARC`) is refused **by
//! name before stitching** by the classifier in the parent module
//! (`hatch.boundary_degenerate`); the stitcher itself only sees edges with a real
//! extent.

use super::*;

/// The absolute endpoint-chaining tolerance, in world units.
///
/// See the module docs: the hatch boundary is model-space world geometry, so an
/// absolute gap is the only honest tolerance here (there is no drawing scale or
/// unit context in this layer to make a relative one meaningful). Endpoints within
/// this distance are the same node; a larger gap is a real opening and is refused.
pub(super) const STITCH_TOLERANCE: f64 = 1e-6;

/// An `ARC` sweep at or below this magnitude is numerically no arc at all.
///
/// The classifier refuses such an edge as `hatch.boundary_degenerate` before
/// stitching; the value matches the tessellator's "no visible arc" slack.
pub(super) const DEGENERATE_SWEEP: f64 = 1e-12;

/// One directed boundary edge offered to the stitcher.
///
/// `start → end` is the entity's own direction (`Arc` sweeps from `start` along
/// its stored sweep); the stitcher may consume it either way round.
#[derive(Debug, Clone, Copy)]
pub(super) struct StitchEdge {
    pub(super) start: Point3,
    pub(super) end: Point3,
}

/// How one loop segment consumes its edge.
#[derive(Debug, Clone, Copy)]
pub(super) struct StitchSegment {
    /// Index into the edge slice passed to [`stitch_edges`].
    pub(super) edge: usize,
    /// `true` when the segment runs `end → start`, i.e. against the stored
    /// direction (an `Arc`'s sweep then flips sign).
    pub(super) reversed: bool,
}

/// One closed loop stitched from `LINE`/`ARC` edges.
#[derive(Debug, Clone)]
pub(super) struct StitchedLoop {
    /// One vertex per segment, in traversal order: vertex `i` is where segment `i`
    /// starts, and the closing segment ends back at vertex 0.
    pub(super) points: Vec<Point3>,
    pub(super) segments: Vec<StitchSegment>,
}

/// Stitch `edges` into closed loops, preserving selection order.
///
/// Returns one loop per edge-connected closed chain, in the order the chains are
/// discovered (the first unconsumed edge seeds each chain). Refusals change
/// nothing and name their reason: `hatch.boundary_branch` (ambiguous node) or
/// `hatch.boundary_open` (an unconsumed open chain).
pub(super) fn stitch_edges(edges: &[StitchEdge]) -> CadResult<Vec<StitchedLoop>> {
    if edges.is_empty() {
        return Ok(Vec::new());
    }

    // 1. Node clustering (`start`, `end`) per edge, in selection order.
    let mut nodes: Vec<Point3> = Vec::new();
    let mut ends: Vec<(usize, usize)> = Vec::with_capacity(edges.len());
    for edge in edges {
        let start = node_index(&mut nodes, edge.start);
        let end = node_index(&mut nodes, edge.end);
        ends.push((start, end));
    }

    // 2. Branch check: more than two edge ends at one node has no unique pairing.
    let mut degree = vec![0usize; nodes.len()];
    for (start, end) in &ends {
        degree[*start] += 1;
        degree[*end] += 1;
    }
    if let Some((node, count)) = degree
        .iter()
        .copied()
        .enumerate()
        .find(|(_, count)| *count > 2)
    {
        let point = nodes[node];
        return Err(CadError::InvalidInput(format!(
            "HATCH boundary stitching is ambiguous: hatch.boundary_branch ({count} edge ends meet \
             at node ({}, {}, {}); a branch point has no unique pairing and is never guessed)",
            point.x, point.y, point.z
        )));
    }

    // Undirected adjacency: `(edge index, true when the edge's start is at node)`.
    let mut incident: Vec<Vec<(usize, bool)>> = vec![Vec::new(); nodes.len()];
    for (index, (start, end)) in ends.iter().enumerate() {
        incident[*start].push((index, true));
        incident[*end].push((index, false));
    }

    // 3./4. Walk every edge-connected component exactly once.
    let mut used = vec![false; edges.len()];
    let mut loops = Vec::new();
    let mut unconsumed = 0usize;
    for seed in 0..edges.len() {
        if used[seed] {
            continue;
        }
        let (segments, closed) = walk_component(seed, &ends, &incident, &mut used);
        if closed {
            let points = segments
                .iter()
                .map(|segment| {
                    let (start, end) = ends[segment.edge];
                    nodes[if segment.reversed { end } else { start }]
                })
                .collect();
            loops.push(StitchedLoop { points, segments });
        } else {
            unconsumed += segments.len();
        }
    }
    if unconsumed > 0 {
        return Err(CadError::InvalidInput(format!(
            "HATCH boundary is not closed: hatch.boundary_open ({unconsumed} unconsumed of {} \
             selected LINE/ARC edge(s) form an open chain; the stitching tolerance is {STITCH_TOLERANCE:e} \
             world units and a larger gap is never bridged)",
            edges.len()
        )));
    }
    Ok(loops)
}

/// The node holding `point`, creating one at `point` when none is close enough.
fn node_index(nodes: &mut Vec<Point3>, point: Point3) -> usize {
    for (index, existing) in nodes.iter().enumerate() {
        if distance(*existing, point) <= STITCH_TOLERANCE {
            return index;
        }
    }
    nodes.push(point);
    nodes.len() - 1
}

/// Walk one edge-connected component, consuming every edge it contains.
///
/// Returns the traversal chain and whether it closed. An open chain is walked in
/// both directions from the seed, so the refusal reports the whole unconsumed
/// chain rather than an arbitrary half of it.
fn walk_component(
    seed: usize,
    ends: &[(usize, usize)],
    incident: &[Vec<(usize, bool)>],
    used: &mut [bool],
) -> (Vec<StitchSegment>, bool) {
    used[seed] = true;
    let start_node = ends[seed].0;
    let mut segments = vec![StitchSegment {
        edge: seed,
        reversed: false,
    }];
    let mut node = ends[seed].1;
    let mut closed = false;
    loop {
        if node == start_node {
            closed = true;
            break;
        }
        let Some(&(edge, at_start)) = incident[node].iter().find(|(edge, _)| !used[*edge]) else {
            break;
        };
        used[edge] = true;
        // Leaving `node` through an edge whose *start* is here traverses it
        // forward; leaving through its end traverses it backwards.
        segments.push(StitchSegment {
            edge,
            reversed: !at_start,
        });
        node = if at_start { ends[edge].1 } else { ends[edge].0 };
    }
    if !closed {
        let mut head = Vec::new();
        let mut node = start_node;
        while let Some(&(edge, at_start)) = incident[node].iter().find(|(edge, _)| !used[*edge]) {
            used[edge] = true;
            // This edge must *end* at `node`: entering it at its start means the
            // chain runs it backwards.
            head.push(StitchSegment {
                edge,
                reversed: at_start,
            });
            node = if at_start { ends[edge].1 } else { ends[edge].0 };
        }
        head.reverse();
        head.extend(segments);
        segments = head;
    }
    (segments, closed)
}
