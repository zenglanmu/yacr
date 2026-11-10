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
//!
//! # Planar face walk (`HATCHPOINT`, v1)
//!
//! HATCH's pre-selection path only needs closed chains; the HATCHPOINT picker has
//! to find the **smallest region that contains a point**, so it also needs the
//! planar subdivision the `LINE`/`ARC` edges form. [`walk_faces`] builds that
//! subdivision exactly:
//!
//! * every visible, in-plane `LINE`/`ARC` is one undirected edge with two
//!   half-edges; endpoints cluster at the same absolute [`STITCH_TOLERANCE`];
//! * dangling chains (a node of degree 1) bound no face and are pruned, so an
//!   unrelated spur — attached or not — never changes the result;
//! * at each node the incident half-edges are sorted by their outgoing **tangent**
//!   angle (`atan2` in the seed plane). A `LINE` tangent is its unit direction; an
//!   `ARC` tangent is perpendicular to the radius in the sweep direction, so both
//!   sweeps and any normal direction are exact and never sampled;
//! * a face is walked by the classic "keep the region on the left" rule: from the
//!   incoming half-edge, take the outgoing half-edge that is the **clockwise
//!   neighbour of its reverse** (the predecessor in the CCW-sorted fan). Walking
//!   every unvisited half-edge enumerates **all** faces, so nested islands are
//!   found, and branch nodes are handled by the fan instead of being refused;
//! * the face that contains the seed is located analytically: a ray from the seed
//!   takes the nearest crossing (line/arc intersection, both exact) and the
//!   half-edge whose left side holds the seed is walked. A ray that hits a vertex
//!   retries another direction. The region's outer boundary is then the
//!   smallest-area positive (counter-clockwise) face whose ring contains the seed —
//!   a region with holes has several boundary cycles, so its outer cycle is
//!   resolved from the enumeration, not guessed;
//! * a face becomes one closed bulged loop: a `LINE` is bulge `0`, an `ARC` is
//!   `tan(sweep / 4)` signed by the traversal direction — exactly the convention
//!   [`stitch_edges`] uses. Nothing is sampled for the topology or the bulge.
//!
//! Declared v1 limits: only `LINE`/`ARC` edges (no `SPLINE` or ellipse-arc edges),
//! only edges on the seed's `+Z` plane at the seed elevation, model space only, no
//! nested block-instance edges, no self-intersecting faces (the
//! `HatchGeometry::new` validator refuses them), and an `ARC` with
//! `|sweep| > 2π` (or a degenerate sweep) is refused as an edge — it is skipped,
//! never sampled or approximated. An edge whose projected endpoints/circle do not
//! agree within the stitching tolerance is likewise skipped; auto-discovery never
//! fails a command because of unrelated or unrepresentable geometry.

use super::hatch_boundary::plane_projection;

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

// ---------------------------------------------------------------------------
// Planar half-edge graph and face walk (HATCHPOINT edge-chain discovery)
// ---------------------------------------------------------------------------

/// Angular slack for the "is this angle inside the arc's swept range" test.
const ARC_RANGE_TOLERANCE: f64 = 1e-9;

/// Angular slack for matching an arc's stored sweep to its two endpoints.
const ARC_ENDPOINT_TOLERANCE: f64 = 1e-6;

/// A ray crossing this close to the seed is the seed itself, not a crossing.
const RAY_MIN_DISTANCE: f64 = 1e-12;

/// How many ray directions point location may try before it gives up.
const RAY_DIRECTIONS: usize = 16;

/// The exact curve of one face-walk edge, in the seed plane.
#[derive(Debug, Clone, Copy)]
pub(super) enum EdgeCurve {
    /// A straight segment: one direction for the whole edge.
    Line,
    /// A circular arc, kept exactly by centre/radius/sweep (never sampled).
    Arc {
        center: Point3,
        radius: f64,
        sweep: f64,
    },
}

/// One undirected boundary edge offered to the planar face walk.
///
/// `start` → `end` is the entity's own direction (an `ARC` sweeps from its start
/// along `sweep`); the walk may consume the edge either way round.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlanarEdge {
    pub(super) start: Point3,
    pub(super) end: Point3,
    pub(super) curve: EdgeCurve,
}

/// One closed face of the planar edge graph, as exact bulge geometry.
#[derive(Debug, Clone)]
pub(super) struct FaceLoop {
    /// One vertex per traversed half-edge, in walk order.
    pub(super) points: Vec<[f64; 2]>,
    /// `tan(sweep / 4)` per segment (`0` for a `LINE`), signed by the traversal.
    pub(super) bulges: Vec<f64>,
    /// Exact signed area: positive when the region lies on the walk's left.
    pub(super) signed_area: f64,
    /// Boundary edges the face consumes (after removing out-and-back spikes).
    pub(super) edge_count: usize,
}

/// The exhaustive face set of one planar edge graph.
pub(super) struct PlanarFaces {
    /// Every face with at least three distinct vertices and a non-zero area.
    pub(super) faces: Vec<FaceLoop>,
    /// The face the analytic ray/walk located around the seed, if any.
    ///
    /// It is the seed's region: a counter-clockwise cycle is that region's own
    /// outer boundary; a region with holes can be met on one of its clockwise hole
    /// cycles, in which case the outer boundary is the smallest positive-area face
    /// whose ring contains the seed (see [`walk_faces`]).
    pub(super) containing: Option<usize>,
}

/// One 2D edge after projection, with its analytic curve and end tangents.
struct PreparedEdge {
    curve: PreparedCurve,
    start: [f64; 2],
    end: [f64; 2],
    /// Unit direction of travel at `start` (`start → end`).
    start_tangent: [f64; 2],
    /// Unit direction of travel at `end` (`start → end`).
    end_tangent: [f64; 2],
}

#[derive(Clone, Copy)]
enum PreparedCurve {
    Line,
    Arc {
        center: [f64; 2],
        radius: f64,
        start_angle: f64,
        sweep: f64,
    },
}

/// One ray/edge crossing: the travelled distance, the point and the forward
/// (stored-direction) tangent there.
struct RayHit {
    t: f64,
    point: [f64; 2],
    tangent: [f64; 2],
}

/// Build the planar subdivision of `edges` and locate the face holding `seed`.
///
/// The seed and the edges must lie in `plane`'s own 2D frame; the caller filters
/// visibility, plane and elevation. A face walk that does not terminate is refused
/// by name (`hatch.boundary_face_degenerate`) because that would be an internal
/// invariant break, not a property of the input.
pub(super) fn walk_faces(
    edges: &[PlanarEdge],
    plane: &HatchPlane,
    seed: Point3,
) -> CadResult<PlanarFaces> {
    let to_2d = plane_projection(plane);
    let prepared = prepare_edges(edges, &to_2d);
    if prepared.len() < 2 {
        return Ok(PlanarFaces {
            faces: Vec::new(),
            containing: None,
        });
    }

    // 1. Cluster endpoints at the absolute stitching tolerance.
    let mut nodes: Vec<[f64; 2]> = Vec::new();
    let mut ends: Vec<(usize, usize)> = Vec::with_capacity(prepared.len());
    for edge in &prepared {
        let start = node_index_2d(&mut nodes, edge.start);
        let end = node_index_2d(&mut nodes, edge.end);
        ends.push((start, end));
    }

    // 2. A degree-1 node ends an open chain, which bounds no face: prune every
    //    dangling chain iteratively. A closed loop always keeps degree >= 2.
    let alive = prune_dangling(&ends);

    // 3. One edge becomes two half-edges (forward = even, reverse = odd), so the
    //    twin of half-edge `h` is always `h ^ 1`.
    let mut half_edge: Vec<usize> = Vec::new();
    let mut half_from: Vec<usize> = Vec::new();
    let mut half_to: Vec<usize> = Vec::new();
    let mut half_angle: Vec<f64> = Vec::new();
    let mut forward_half: Vec<Option<usize>> = vec![None; prepared.len()];
    for (index, edge) in prepared.iter().enumerate() {
        if !alive[index] {
            continue;
        }
        let (start, end) = ends[index];
        forward_half[index] = Some(half_edge.len());
        half_edge.push(index);
        half_from.push(start);
        half_to.push(end);
        half_angle.push(edge.start_tangent[1].atan2(edge.start_tangent[0]));
        half_edge.push(index);
        half_from.push(end);
        half_to.push(start);
        half_angle.push((-edge.end_tangent[1]).atan2(-edge.end_tangent[0]));
    }
    let half_count = half_edge.len();
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for half in 0..half_count {
        incident[half_from[half]].push(half);
    }
    for list in &mut incident {
        list.sort_by(|a, b| half_angle[*a].total_cmp(&half_angle[*b]));
    }

    // 4. Walk every unvisited half-edge: each walk is one boundary cycle of a face.
    let mut visited = vec![false; half_count];
    let mut cycle_of_half = vec![usize::MAX; half_count];
    let mut cycles: Vec<Vec<usize>> = Vec::new();
    for start in 0..half_count {
        if visited[start] {
            continue;
        }
        let cycle_index = cycles.len();
        let mut cycle = Vec::new();
        let mut half = start;
        loop {
            if visited[half] || cycle.len() > half_count {
                return Err(degenerate_face_refusal());
            }
            visited[half] = true;
            cycle_of_half[half] = cycle_index;
            cycle.push(half);
            half = next_half_edge(half, &half_to, &incident);
            if half == start {
                break;
            }
        }
        cycles.push(remove_spikes(cycle));
    }

    // 5. Keep the closed, non-degenerate faces and remember the half → face map.
    let mut faces = Vec::new();
    let mut face_of_cycle: Vec<Option<usize>> = vec![None; cycles.len()];
    for (index, cycle) in cycles.iter().enumerate() {
        if let Some(face) = face_loop(cycle, &half_edge, &half_from, &nodes, &prepared) {
            face_of_cycle[index] = Some(faces.len());
            faces.push(face);
        }
    }
    let mut face_of_half = vec![usize::MAX; half_count];
    for (half, cycle) in cycle_of_half.iter().copied().enumerate() {
        if let Some(face) = face_of_cycle[cycle] {
            face_of_half[half] = face;
        }
    }

    // 6. Locate the seed: the ray finds the half-edge whose left face holds it and
    //    the walk of that half-edge names the face. A positive (counter-clockwise)
    //    cycle is the region's own outer boundary; a negative cycle is the hole
    //    side of a region that has several boundary cycles, so its outer boundary
    //    is resolved as the smallest positive face that contains the seed.
    let seed_2d = to_2d(seed);
    let ray_face = locate_half(seed_2d, &prepared, &nodes, &forward_half)
        .map(|half| face_of_half[half])
        .filter(|face| *face != usize::MAX);
    let containing = match ray_face {
        None => None,
        Some(face) if faces[face].signed_area > 0.0 => Some(face),
        Some(_) => {
            let mut best: Option<usize> = None;
            for (index, face) in faces.iter().enumerate() {
                if face.signed_area <= 0.0 {
                    continue;
                }
                if !cad_domain::hatch::point_in_ring(seed_2d, &face_ring(face)) {
                    continue;
                }
                if best.is_none_or(|current: usize| face.signed_area < faces[current].signed_area) {
                    best = Some(index);
                }
            }
            best
        }
    };
    Ok(PlanarFaces { faces, containing })
}

/// Project and validate every edge; an unrepresentable edge is skipped.
fn prepare_edges(edges: &[PlanarEdge], to_2d: &impl Fn(Point3) -> [f64; 2]) -> Vec<PreparedEdge> {
    let mut prepared = Vec::with_capacity(edges.len());
    for edge in edges {
        let start = to_2d(edge.start);
        let end = to_2d(edge.end);
        if !finite_2d(start) || !finite_2d(end) || distance_2d(start, end) <= STITCH_TOLERANCE {
            continue;
        }
        match edge.curve {
            EdgeCurve::Line => {
                let Some(direction) = unit_2d(sub_2d(end, start)) else {
                    continue;
                };
                prepared.push(PreparedEdge {
                    curve: PreparedCurve::Line,
                    start,
                    end,
                    start_tangent: direction,
                    end_tangent: direction,
                });
            }
            EdgeCurve::Arc {
                center,
                radius,
                sweep,
            } => {
                let center = to_2d(center);
                if !finite_2d(center)
                    || !radius.is_finite()
                    || radius <= STITCH_TOLERANCE
                    || !sweep.is_finite()
                    || sweep.abs() <= DEGENERATE_SWEEP
                    || sweep.abs() > std::f64::consts::TAU + 1e-9
                {
                    continue;
                }
                // The projected endpoints must sit on one projected circle, and the
                // stored sweep must take the start angle to the end angle; otherwise
                // this "arc" is not a circular arc in the seed frame and is skipped
                // instead of being approximated.
                let start_radius = distance_2d(start, center);
                let end_radius = distance_2d(end, center);
                let radius_2d = 0.5 * (start_radius + end_radius);
                if radius_2d <= STITCH_TOLERANCE
                    || (start_radius - end_radius).abs() > ARC_ENDPOINT_TOLERANCE * radius_2d
                {
                    continue;
                }
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                if angle_delta(start_angle + sweep, end_angle).abs() > ARC_ENDPOINT_TOLERANCE {
                    continue;
                }
                let sign = sweep.signum();
                let start_tangent = [-sign * start_angle.sin(), sign * start_angle.cos()];
                let swept_angle = start_angle + sweep;
                let end_tangent = [-sign * swept_angle.sin(), sign * swept_angle.cos()];
                prepared.push(PreparedEdge {
                    curve: PreparedCurve::Arc {
                        center,
                        radius: radius_2d,
                        start_angle,
                        sweep,
                    },
                    start,
                    end,
                    start_tangent,
                    end_tangent,
                });
            }
        }
    }
    prepared
}

/// The node holding `point`, creating one at `point` when none is close enough.
fn node_index_2d(nodes: &mut Vec<[f64; 2]>, point: [f64; 2]) -> usize {
    for (index, existing) in nodes.iter().enumerate() {
        if distance_2d(*existing, point) <= STITCH_TOLERANCE {
            return index;
        }
    }
    nodes.push(point);
    nodes.len() - 1
}

/// Mark every edge that belongs to a dangling chain (iteratively).
///
/// A degree-1 node cannot be part of a face boundary, so its edge is removed and
/// the degrees are recomputed. Closed loops always survive; spurs (attached or
/// free-standing) disappear before any face is walked.
fn prune_dangling(ends: &[(usize, usize)]) -> Vec<bool> {
    let node_count = ends
        .iter()
        .flat_map(|(start, end)| [*start, *end])
        .max()
        .map_or(0, |node| node + 1);
    let mut alive = vec![true; ends.len()];
    loop {
        let mut degree = vec![0usize; node_count];
        for (index, (start, end)) in ends.iter().enumerate() {
            if !alive[index] {
                continue;
            }
            degree[*start] += 1;
            degree[*end] += 1;
        }
        let mut changed = false;
        for (index, (start, end)) in ends.iter().enumerate() {
            if alive[index] && (degree[*start] < 2 || degree[*end] < 2) {
                alive[index] = false;
                changed = true;
            }
        }
        if !changed {
            return alive;
        }
    }
}

/// The next half-edge of the face on the left: the clockwise neighbour of the
/// reverse half-edge in the node's outgoing fan.
fn next_half_edge(half: usize, half_to: &[usize], incident: &[Vec<usize>]) -> usize {
    let twin = half ^ 1;
    let fan = &incident[half_to[half]];
    let position = fan
        .iter()
        .position(|candidate| *candidate == twin)
        .expect("every live half-edge's twin is incident to its head node");
    fan[(position + fan.len() - 1) % fan.len()]
}

/// Remove out-and-back spike pairs (an edge immediately consumed backwards).
///
/// A bridge or a spur traversed twice contributes no area and no boundary; the
/// pair is dropped so the face ring stays a closed bulge loop.
fn remove_spikes(mut cycle: Vec<usize>) -> Vec<usize> {
    loop {
        let count = cycle.len();
        let mut removed = false;
        for index in 0..count {
            let next = (index + 1) % count;
            if cycle[next] == (cycle[index] ^ 1) {
                let (low, high) = if index < next {
                    (index, next)
                } else {
                    (next, index)
                };
                cycle.remove(high);
                cycle.remove(low);
                removed = true;
                break;
            }
        }
        if !removed || cycle.len() < 3 {
            return cycle;
        }
    }
}

/// Convert one boundary cycle into exact bulge geometry, or refuse it as degenerate.
///
/// A face with only two graph edges (an `ARC` closed by one straight chord) is the
/// thinnest possible lens; the creation-path validator needs three vertices, so
/// that single arc is split **exactly** into two sub-arcs (same circle,
/// `tan(sweep / 8)` each). Nothing is sampled, and every other face keeps one exact
/// segment per graph edge.
fn face_loop(
    cycle: &[usize],
    half_edge: &[usize],
    half_from: &[usize],
    nodes: &[[f64; 2]],
    prepared: &[PreparedEdge],
) -> Option<FaceLoop> {
    // A single half-edge is a self-loop (a full-turn arc) and bounds no lens; two
    // half-edges are the arc + chord lens handled by the exact split below.
    if cycle.len() < 2 {
        return None;
    }
    let mut points = Vec::with_capacity(cycle.len() + 1);
    let mut bulges = Vec::with_capacity(cycle.len() + 1);
    let mut split_done = false;
    for (index, half) in cycle.iter().copied().enumerate() {
        let start = nodes[half_from[half]];
        let end = nodes[half_from[cycle[(index + 1) % cycle.len()]]];
        // A zero-length segment makes the bulge meaningless and the loop invalid.
        if distance_2d(start, end) <= STITCH_TOLERANCE {
            return None;
        }
        points.push(start);
        let (bulge, split) = match prepared[half_edge[half]].curve {
            PreparedCurve::Line => (0.0, None),
            PreparedCurve::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => {
                // The traversal direction of this half-edge: forward keeps the
                // stored sweep, reverse starts at the end and negates it.
                let (angle, sweep) = if half.is_multiple_of(2) {
                    (start_angle, sweep)
                } else {
                    (start_angle + sweep, -sweep)
                };
                if cycle.len() < 3 && !split_done {
                    split_done = true;
                    let middle_angle = angle + sweep / 2.0;
                    let middle = [
                        center[0] + radius * middle_angle.cos(),
                        center[1] + radius * middle_angle.sin(),
                    ];
                    let sub_bulge = (sweep / 8.0).tan();
                    (sub_bulge, Some((middle, sub_bulge)))
                } else {
                    ((sweep / 4.0).tan(), None)
                }
            }
        };
        bulges.push(bulge);
        if let Some((middle, sub_bulge)) = split {
            points.push(middle);
            bulges.push(sub_bulge);
        }
    }
    if points.len() < 3 {
        return None;
    }
    let signed_area = exact_signed_area(&points, &bulges);
    if !signed_area.is_finite() || signed_area.abs() <= f64::MIN_POSITIVE {
        return None;
    }
    Some(FaceLoop {
        points,
        bulges,
        signed_area,
        edge_count: cycle.len(),
    })
}

/// The exact signed area of a bulged loop: chord shoelace plus every circular
/// segment. Used for orientation and area comparisons only; the fill keeps the
/// loop's points and bulges.
fn exact_signed_area(points: &[[f64; 2]], bulges: &[f64]) -> f64 {
    let count = points.len();
    let mut area = 0.0;
    for index in 0..count {
        let a = points[index];
        let b = points[(index + 1) % count];
        area += 0.5 * (a[0] * b[1] - b[0] * a[1]);
    }
    for index in 0..count {
        let bulge = bulges[index];
        if bulge == 0.0 || !bulge.is_finite() {
            continue;
        }
        let sweep = 4.0 * bulge.atan();
        let a = points[index];
        let b = points[(index + 1) % count];
        let sin_half = (sweep / 2.0).sin();
        if sin_half.abs() <= 1e-12 {
            continue;
        }
        let radius = distance_2d(a, b) / (2.0 * sin_half.abs());
        area += 0.5 * radius * radius * (sweep - sweep.sin());
    }
    area
}

/// Sample a face ring for containment tests only (the bulge geometry itself is
/// never sampled here). Delegates to the single domain sampler, so a face ring
/// and a stored hatch loop use the same tessellation.
fn face_ring(face: &FaceLoop) -> Vec<[f64; 2]> {
    HatchLoop::new(
        face.points.clone(),
        face.bulges.clone(),
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    )
    .sampled_ring()
}

/// Locate the half-edge whose left face holds `seed`, by analytic ray casting.
///
/// The first direction that produces an unambiguous crossing wins (a hit on a
/// vertex, or a grazing hit, retries the next direction); the nearest crossing is
/// the one the ray leaves the seed's region through, and the half-edge whose
/// forward tangent keeps the seed on its left is that region's boundary edge.
fn locate_half(
    seed: [f64; 2],
    prepared: &[PreparedEdge],
    nodes: &[[f64; 2]],
    forward_half: &[Option<usize>],
) -> Option<usize> {
    for step in 0..RAY_DIRECTIONS {
        let angle = step as f64 * std::f64::consts::TAU / RAY_DIRECTIONS as f64;
        let direction = [angle.cos(), angle.sin()];
        let mut hits: Vec<(f64, usize, RayHit)> = Vec::new();
        for (index, edge) in prepared.iter().enumerate() {
            if forward_half[index].is_none() {
                continue;
            }
            for hit in ray_hits(edge, seed, direction) {
                hits.push((hit.t, index, hit));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, index, hit)) = hits.first() else {
            continue;
        };
        if nodes
            .iter()
            .any(|node| distance_2d(*node, hit.point) <= STITCH_TOLERANCE)
        {
            continue;
        }
        let side = cross_2d(hit.tangent, sub_2d(seed, hit.point));
        if side.abs() <= 1e-12 {
            continue;
        }
        let forward = forward_half[*index].expect("checked above");
        return Some(if side > 0.0 { forward } else { forward ^ 1 });
    }
    None
}

/// Every analytic crossing of the ray `seed + t * direction` with one edge.
fn ray_hits(edge: &PreparedEdge, seed: [f64; 2], direction: [f64; 2]) -> Vec<RayHit> {
    match edge.curve {
        PreparedCurve::Line => {
            let span = sub_2d(edge.end, edge.start);
            let denominator = cross_2d(direction, span);
            if denominator.abs() <= 1e-12 {
                return Vec::new();
            }
            let offset = sub_2d(edge.start, seed);
            let t = cross_2d(offset, span) / denominator;
            if t <= RAY_MIN_DISTANCE {
                return Vec::new();
            }
            let u = cross_2d(offset, direction) / denominator;
            if !(0.0..=1.0).contains(&u) {
                return Vec::new();
            }
            vec![RayHit {
                t,
                point: [seed[0] + t * direction[0], seed[1] + t * direction[1]],
                tangent: edge.start_tangent,
            }]
        }
        PreparedCurve::Arc {
            center,
            radius,
            start_angle,
            sweep,
        } => {
            let offset = sub_2d(seed, center);
            let projection = dot_2d(offset, direction);
            let constant = dot_2d(offset, offset) - radius * radius;
            let discriminant = projection * projection - constant;
            if discriminant <= 0.0 {
                return Vec::new();
            }
            let root = discriminant.sqrt();
            let mut hits = Vec::new();
            for t in [-projection - root, -projection + root] {
                if t <= RAY_MIN_DISTANCE {
                    continue;
                }
                let point = [seed[0] + t * direction[0], seed[1] + t * direction[1]];
                let theta = (point[1] - center[1]).atan2(point[0] - center[0]);
                if !angle_within(theta, start_angle, sweep) {
                    continue;
                }
                let sign = sweep.signum();
                hits.push(RayHit {
                    t,
                    point,
                    tangent: [-sign * theta.sin(), sign * theta.cos()],
                });
            }
            hits
        }
    }
}

/// Whether `angle` lies on the arc that starts at `start` and sweeps `sweep`.
pub(super) fn angle_within(angle: f64, start: f64, sweep: f64) -> bool {
    if !angle.is_finite() || !start.is_finite() || !sweep.is_finite() {
        return false;
    }
    let mut delta = (angle - start) % std::f64::consts::TAU;
    if delta < 0.0 {
        delta += std::f64::consts::TAU;
    }
    if sweep >= 0.0 {
        delta <= sweep + ARC_RANGE_TOLERANCE || delta >= std::f64::consts::TAU - ARC_RANGE_TOLERANCE
    } else {
        delta >= std::f64::consts::TAU + sweep - ARC_RANGE_TOLERANCE || delta <= ARC_RANGE_TOLERANCE
    }
}

fn degenerate_face_refusal() -> CadError {
    CadError::InvalidInput(
        "HATCHPOINT boundary face walk did not terminate: hatch.boundary_face_degenerate (the \
         LINE/ARC half-edge fan is inconsistent; no face is guessed)"
            .into(),
    )
}

fn finite_2d(point: [f64; 2]) -> bool {
    point[0].is_finite() && point[1].is_finite()
}

fn sub_2d(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot_2d(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross_2d(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn distance_2d(a: [f64; 2], b: [f64; 2]) -> f64 {
    sub_2d(a, b)[0].hypot(sub_2d(a, b)[1])
}

fn unit_2d(v: [f64; 2]) -> Option<[f64; 2]> {
    let length = v[0].hypot(v[1]);
    if !length.is_finite() || length <= 1e-12 {
        None
    } else {
        Some([v[0] / length, v[1] / length])
    }
}

/// Wrap `angle` into `(-π, π]`.
fn angle_delta(a: f64, b: f64) -> f64 {
    let mut delta = (a - b) % std::f64::consts::TAU;
    if delta > std::f64::consts::PI {
        delta -= std::f64::consts::TAU;
    }
    if delta <= -std::f64::consts::PI {
        delta += std::f64::consts::TAU;
    }
    delta
}
