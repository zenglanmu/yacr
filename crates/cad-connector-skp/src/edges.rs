//! Real SketchUp `MeshEdge` records: visibility, canonical endpoints, and the
//! face-boundary / standalone-wire split (ADR-0049).
//!
//! `skpwright` materializes one `MeshEdge` per SketchUp edge, with a run-local
//! vertex pair plus soft/smooth/hidden flags, a tag (layer) slot, a material
//! slot and an optional curve membership. This module is the only place that
//! decides which of those records become document geometry:
//!
//! * a **face/material group** may move to [`crate::convert::PlacedMesh`]'s
//!   body carrier only when **every** consecutive pair of every visible face
//!   ring has a matching record — otherwise the group keeps the mesh carrier and
//!   ADR-0045 derives its wireframe from the triangles;
//! * visible records **not** covered by a visible face boundary (loose line
//!   work, and edges of faces in a group that produced no mesh triangles)
//!   become standalone wire polylines, grouped by the edge's own effective
//!   material.
//!
//! Everything here is a filter or an index; nothing invents a coordinate. The
//! stored polylines are always the edge's own two endpoints, transformed by the
//! instance's world matrix — a curve is its chord (`curve` is counted, not
//! evaluated), and a `smooth` edge keeps no normal-smoothing state.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::Point3;
use skpwright::{Mesh, MeshEdge, MeshFace};

use crate::convert::apply_point;

/// A canonical (unordered) pair of run-local vertex indices.
pub(crate) type VertexPair = (u32, u32);

/// Order a pair so `(a, b)` and `(b, a)` name the same edge.
pub(crate) fn canonical(v0: u32, v1: u32) -> VertexPair {
    if v0 <= v1 {
        (v0, v1)
    } else {
        (v1, v0)
    }
}

/// Every counter the edge pass reports. Each skipped record lands in exactly
/// one bucket, and every stored segment is counted, so nothing is a silent drop.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EdgeStats {
    /// `MeshEdge` records in every placed run.
    pub records: usize,
    /// Records pruned because the edge itself is hidden.
    pub hidden_skipped: usize,
    /// Records pruned because the edge is soft (SketchUp hides it at a
    /// camera-facing rule this preview does not model).
    pub soft_skipped: usize,
    /// Records pruned because their tag (layer) is invisible.
    pub invisible_layer_skipped: usize,
    /// Records pruned because their two endpoints are the same point.
    pub zero_length_skipped: usize,
    /// Records pruned because an endpoint index does not name a vertex.
    pub index_skipped: usize,
    /// Records pruned because the group already stores the same segment.
    pub duplicate_skipped: usize,
    /// Records that carry a flag or semantic this preview does not model
    /// (hidden/soft/smooth, a per-edge material, a shadow flag set off).
    ///
    /// The `edge_flags_not_modeled` diagnostic fires when this is non-zero, or
    /// when `zero_length_skipped`/`index_skipped` dropped a malformed record: a
    /// file whose records are all plain visible edges has nothing to report, so
    /// the note stays silent.
    pub flag_records: usize,
    /// Stored 2-point polylines (body boundaries plus standalone wires).
    pub emitted: usize,
    /// Stored polylines that are a face boundary on a body.
    pub boundary: usize,
    /// Stored polylines that are standalone (uncovered) wire edges.
    pub wire: usize,
    /// Stored polylines whose record belongs to a curve; the chord is stored.
    pub curves_flattened: usize,
    /// Visible records whose line is already drawn because the record was stored
    /// on a preview body's `BodyDisplay.edges` (a face-ring boundary), so no
    /// standalone wire is stored for it.
    pub covered_by_body: usize,
    /// Visible records whose line is already drawn by a mesh group's derived
    /// surface boundary (ADR-0045), so no document edge is stored for them.
    pub covered_by_mesh: usize,
    /// Groups that kept the plain mesh carrier because a ring segment had no
    /// `MeshEdge` record.
    pub groups_without_records: usize,
    /// Ring segments of those groups that have no record (the evidence behind
    /// `groups_without_records`).
    pub unrecorded_boundary_segments: usize,
    /// Groups that had complete records but whose visible boundary was empty
    /// after filtering, so they kept the mesh carrier (never an edge-less body).
    pub groups_empty_boundary: usize,
}

impl EdgeStats {
    /// Records pruned for any reason (the `hidden_pruned` sum).
    pub fn skipped(&self) -> usize {
        self.hidden_skipped
            + self.soft_skipped
            + self.invisible_layer_skipped
            + self.zero_length_skipped
            + self.index_skipped
            + self.duplicate_skipped
    }
}

/// The outcome of asking a group's faces for their real boundary edges.
#[derive(Debug)]
pub(crate) enum Boundary {
    /// Every ring segment is backed by a `MeshEdge` record. `curves` counts the
    /// stored segments whose record names a curve (flattened to its chord).
    Complete {
        edges: Vec<Vec<Point3>>,
        curves: usize,
    },
    /// At least one ring segment has no `MeshEdge` record; the group keeps the
    /// mesh carrier. The count is the number of uncovered segments.
    MissingRecords(usize),
}

/// One standalone wire group's stored polylines.
#[derive(Debug, Default)]
pub(crate) struct WireEdges {
    pub edges: Vec<Vec<Point3>>,
    /// Stored segments whose record belongs to a curve.
    pub curves: usize,
    /// Duplicate records dropped inside this group.
    pub duplicates: usize,
    /// Segments whose world endpoints coincide (a collapsing instance).
    pub degenerate: usize,
}

/// One run's edge records, split into "proves a face boundary" and "visible".
pub(crate) struct RunEdges<'a> {
    /// The first record for each canonical pair, used to prove that a face ring
    /// segment really is a SketchUp edge. Hidden/soft/invisible records stay
    /// here: the edge exists, it is only not drawn.
    records: BTreeMap<VertexPair, &'a MeshEdge>,
    /// Records that survive the visibility filter, in file order.
    visible: Vec<(&'a MeshEdge, VertexPair)>,
    /// Canonical pairs of `visible`, for the boundary emission test.
    visible_pairs: BTreeSet<VertexPair>,
}

impl<'a> RunEdges<'a> {
    /// Index one placed run's records. `layer_visible` is the walk's own tag
    /// visibility rule, so an edge and a face on the same tag agree.
    pub(crate) fn new(
        mesh: &'a Mesh,
        layer_visible: &dyn Fn(u16) -> bool,
        stats: &mut EdgeStats,
    ) -> Self {
        let mut records = BTreeMap::new();
        let mut visible = Vec::new();
        let mut visible_pairs = BTreeSet::new();
        for edge in &mesh.edges {
            stats.records += 1;
            if edge.hidden
                || edge.soft
                || edge.smooth
                || edge.material.is_some()
                || !edge.casts_shadows
                || !edge.receives_shadows
            {
                stats.flag_records += 1;
            }
            if edge.v0 == edge.v1 {
                stats.zero_length_skipped += 1;
                continue;
            }
            let (Some(v0), Some(v1)) = (
                mesh.vertices.get(edge.v0 as usize),
                mesh.vertices.get(edge.v1 as usize),
            ) else {
                stats.index_skipped += 1;
                continue;
            };
            let pair = canonical(edge.v0, edge.v1);
            records.entry(pair).or_insert(edge);
            if edge.hidden {
                stats.hidden_skipped += 1;
                continue;
            }
            if edge.soft {
                stats.soft_skipped += 1;
                continue;
            }
            if !layer_visible(edge.layer) {
                stats.invisible_layer_skipped += 1;
                continue;
            }
            if same_point(*v0, *v1) {
                stats.zero_length_skipped += 1;
                continue;
            }
            visible.push((edge, pair));
            visible_pairs.insert(pair);
        }
        RunEdges {
            records,
            visible,
            visible_pairs,
        }
    }

    /// The real boundary of `faces`, in world metres.
    ///
    /// A group may only take the body carrier when every ring segment of every
    /// visible face has a record. When it can, the stored set is the records
    /// that passed the visibility filter — an edge SketchUp hides is not drawn
    /// here either, so the set may be smaller than the rings (or empty, which
    /// the caller treats as "keep the mesh carrier").
    pub(crate) fn boundary(&self, mesh: &Mesh, faces: &[&MeshFace], world: &[f64; 16]) -> Boundary {
        let required = required_pairs(faces);
        let missing = required
            .iter()
            .filter(|pair| !self.records.contains_key(pair))
            .count();
        if missing > 0 {
            return Boundary::MissingRecords(missing);
        }
        let mut edges = Vec::with_capacity(required.len());
        let mut curves = 0;
        for pair in &required {
            if !self.visible_pairs.contains(pair) {
                continue;
            }
            let Some(record) = self.records.get(pair) else {
                continue;
            };
            let (Some(a), Some(b)) = (
                mesh.vertices.get(pair.0 as usize),
                mesh.vertices.get(pair.1 as usize),
            ) else {
                continue;
            };
            if record.curve.is_some() {
                curves += 1;
            }
            edges.push(world_segment(world, *a, *b));
        }
        Boundary::Complete { edges, curves }
    }

    /// Visible records not covered by a visible face boundary, grouped by the
    /// edge's effective material slot (`0` = none).
    ///
    /// `covered_by_body` is the run's union of every face-ring pair a preview
    /// body already stores (its `BodyDisplay.edges`); `covered_by_mesh` is the
    /// union of the rings a mesh-carrier group actually triangulated. Every
    /// covered record is counted in exactly one bucket, so the two numbers never
    /// both claim the same drawn line. A record used twice within one group is
    /// stored once; the duplicate is counted.
    pub(crate) fn wires(
        &self,
        mesh: &Mesh,
        covered_by_body: &BTreeSet<VertexPair>,
        covered_by_mesh: &BTreeSet<VertexPair>,
        world: &[f64; 16],
        stats: &mut EdgeStats,
    ) -> BTreeMap<u16, WireEdges> {
        let mut groups: BTreeMap<u16, WireEdges> = BTreeMap::new();
        let mut seen: BTreeMap<u16, BTreeSet<VertexPair>> = BTreeMap::new();
        for (record, pair) in &self.visible {
            if covered_by_body.contains(pair) {
                stats.covered_by_body += 1;
                continue;
            }
            if covered_by_mesh.contains(pair) {
                stats.covered_by_mesh += 1;
                continue;
            }
            let (Some(a), Some(b)) = (
                mesh.vertices.get(pair.0 as usize),
                mesh.vertices.get(pair.1 as usize),
            ) else {
                continue;
            };
            let material = record.material.unwrap_or(0);
            let group = groups.entry(material).or_default();
            if !seen.entry(material).or_default().insert(*pair) {
                group.duplicates += 1;
                continue;
            }
            let start = apply_point(world, *a);
            let end = apply_point(world, *b);
            if same_point(start, end) {
                group.degenerate += 1;
                continue;
            }
            if record.curve.is_some() {
                group.curves += 1;
            }
            group.edges.push(vec![point(start), point(end)]);
        }
        groups
    }
}

/// Every canonical ring pair of these faces (outer loops and holes).
pub(crate) fn required_pairs(faces: &[&MeshFace]) -> BTreeSet<VertexPair> {
    let mut pairs = BTreeSet::new();
    for face in faces {
        for ring in std::iter::once(&face.outer).chain(face.holes.iter()) {
            if ring.len() < 2 {
                continue;
            }
            for index in 0..ring.len() {
                let a = ring[index];
                let b = ring[(index + 1) % ring.len()];
                pairs.insert(canonical(a, b));
            }
        }
    }
    pairs
}

/// The ring pairs of one face, for the mesh carrier's coverage set.
pub(crate) fn face_ring_pairs(face: &MeshFace) -> BTreeSet<VertexPair> {
    required_pairs(&[face])
}

fn world_segment(world: &[f64; 16], a: [f64; 3], b: [f64; 3]) -> Vec<Point3> {
    vec![point(apply_point(world, a)), point(apply_point(world, b))]
}

fn point(value: [f64; 3]) -> Point3 {
    Point3 {
        x: value[0],
        y: value[1],
        z: value[2],
    }
}

fn same_point(a: [f64; 3], b: [f64; 3]) -> bool {
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [f64; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    fn quad_vertices() -> Vec<[f64; 3]> {
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]
    }

    fn edge(v0: u32, v1: u32) -> MeshEdge {
        MeshEdge {
            v0,
            v1,
            ..Default::default()
        }
    }

    fn quad_ring() -> MeshFace {
        MeshFace {
            outer: vec![0, 1, 2, 3],
            normal: [0.0, 0.0, 1.0],
            ..Default::default()
        }
    }

    fn mesh(edges: Vec<MeshEdge>) -> Mesh {
        Mesh {
            vertices: quad_vertices(),
            edges,
            ..Default::default()
        }
    }

    fn visible(_: u16) -> bool {
        true
    }

    #[test]
    fn a_complete_quad_boundary_is_the_four_ring_segments() {
        let run = mesh(vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 0)]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        let face = quad_ring();
        let Boundary::Complete { edges, curves } = index.boundary(&run, &[&face], &IDENTITY) else {
            panic!("every ring segment has a record");
        };
        assert_eq!(edges.len(), 4);
        assert_eq!(curves, 0);
        // No triangle diagonal: every stored segment joins two ring neighbours.
        for segment in &edges {
            let length = ((segment[1].x - segment[0].x).powi(2)
                + (segment[1].y - segment[0].y).powi(2))
            .sqrt();
            assert!((length - 1.0).abs() < 1e-9, "chord length {length}");
        }
        assert_eq!(stats.records, 4);
        assert_eq!(stats.skipped(), 0);
    }

    #[test]
    fn an_unordered_pair_names_one_edge() {
        let run = mesh(vec![edge(1, 0), edge(2, 1), edge(3, 2), edge(0, 3)]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        let face = quad_ring();
        assert!(matches!(
            index.boundary(&run, &[&face], &IDENTITY),
            Boundary::Complete { edges, .. } if edges.len() == 4
        ));
    }

    #[test]
    fn a_ring_segment_without_a_record_is_reported() {
        let run = mesh(vec![edge(0, 1), edge(1, 2), edge(2, 3)]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        let face = quad_ring();
        assert!(matches!(
            index.boundary(&run, &[&face], &IDENTITY),
            Boundary::MissingRecords(1)
        ));
    }

    #[test]
    fn hidden_soft_and_invisible_tag_edges_are_pruned_and_counted() {
        let mut hidden = edge(0, 1);
        hidden.hidden = true;
        let mut soft = edge(1, 2);
        soft.soft = true;
        let mut on_invisible_tag = edge(2, 3);
        on_invisible_tag.layer = 9;
        let run = mesh(vec![hidden, soft, on_invisible_tag, edge(3, 0)]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &|slot| slot != 9, &mut stats);
        assert_eq!(stats.hidden_skipped, 1);
        assert_eq!(stats.soft_skipped, 1);
        assert_eq!(stats.invisible_layer_skipped, 1);
        assert_eq!(stats.records, 4);
        let face = quad_ring();
        // The records still exist, so the group qualifies; but only the one
        // visible segment is stored.
        let Boundary::Complete { edges, .. } = index.boundary(&run, &[&face], &IDENTITY) else {
            panic!("records exist for every ring segment");
        };
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn a_boundary_whose_records_are_all_pruned_stores_nothing() {
        let mut hidden = edge(0, 1);
        hidden.hidden = true;
        let mut soft = edge(1, 2);
        soft.soft = true;
        let mut hidden2 = edge(2, 3);
        hidden2.hidden = true;
        let mut hidden3 = edge(3, 0);
        hidden3.hidden = true;
        let run = mesh(vec![hidden, soft, hidden2, hidden3]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        let face = quad_ring();
        assert!(matches!(
            index.boundary(&run, &[&face], &IDENTITY),
            Boundary::Complete { edges, .. } if edges.is_empty()
        ));
    }

    #[test]
    fn a_degenerate_record_is_counted_and_not_stored() {
        let run = mesh(vec![
            edge(1, 1),
            // Two distinct indices at the same coordinate.
            edge(0, 4),
            edge(0, 1),
        ]);
        let mut vertices = quad_vertices();
        vertices.push([0.0, 0.0, 0.0]);
        let run = Mesh { vertices, ..run };
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        assert_eq!(stats.zero_length_skipped, 2);
        assert_eq!(stats.records, 3);
        let covered = BTreeSet::new();
        let wires = index.wires(&run, &BTreeSet::new(), &covered, &IDENTITY, &mut stats);
        // Only the real unit edge survives, as an un-materialled wire.
        assert_eq!(wires[&0].edges.len(), 1);
    }

    #[test]
    fn an_out_of_range_endpoint_is_counted_not_indexed() {
        let run = mesh(vec![edge(0, 99)]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        assert_eq!(stats.index_skipped, 1);
        assert_eq!(stats.records, 1);
        assert!(index
            .wires(
                &run,
                &BTreeSet::new(),
                &BTreeSet::new(),
                &IDENTITY,
                &mut stats
            )
            .is_empty());
    }

    #[test]
    fn uncovered_edges_become_wires_grouped_by_material() {
        let mut painted = edge(2, 3);
        painted.material = Some(7);
        let run = mesh(vec![edge(0, 1), painted]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        let mut covered = BTreeSet::new();
        covered.insert(canonical(0, 1));
        let wires = index.wires(&run, &BTreeSet::new(), &covered, &IDENTITY, &mut stats);
        assert_eq!(wires.len(), 1, "only the painted edge is uncovered");
        let group = &wires[&7];
        assert_eq!(group.edges.len(), 1);
        assert_eq!(group.edges[0][0].x, 1.0);
        assert_eq!(group.edges[0][1].y, 1.0);
    }

    #[test]
    fn a_duplicate_record_inside_one_group_is_stored_once() {
        let mut first = edge(0, 1);
        first.material = Some(2);
        let mut second = edge(1, 0);
        second.material = Some(2);
        let run = mesh(vec![first, second]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        let wires = index.wires(
            &run,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &IDENTITY,
            &mut stats,
        );
        assert_eq!(wires[&2].edges.len(), 1);
        assert_eq!(wires[&2].duplicates, 1);
    }

    #[test]
    fn a_curve_edge_keeps_its_chord_and_is_counted() {
        let mut curved = edge(0, 1);
        curved.curve = Some(0);
        let run = mesh(vec![curved]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        let wires = index.wires(
            &run,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &IDENTITY,
            &mut stats,
        );
        assert_eq!(wires[&0].curves, 1);
        assert_eq!(wires[&0].edges.len(), 1);
        assert_eq!(wires[&0].edges[0][0].x, 0.0);
        assert_eq!(wires[&0].edges[0][1].x, 1.0);
    }

    #[test]
    fn the_instance_transform_moves_both_endpoints() {
        let world = [
            1.0, 0.0, 0.0, 10.0, //
            0.0, 1.0, 0.0, 20.0, //
            0.0, 0.0, 1.0, 30.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let run = mesh(vec![edge(0, 1)]);
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        let wires = index.wires(&run, &BTreeSet::new(), &BTreeSet::new(), &world, &mut stats);
        let segment = &wires[&0].edges[0];
        assert_eq!(
            (segment[0].x, segment[0].y, segment[0].z),
            (10.0, 20.0, 30.0)
        );
        assert_eq!(
            (segment[1].x, segment[1].y, segment[1].z),
            (11.0, 20.0, 30.0)
        );
    }

    #[test]
    fn a_hole_ring_must_have_records_too() {
        let run = mesh(vec![
            edge(0, 1),
            edge(1, 2),
            edge(2, 3),
            edge(3, 0),
            edge(0, 1),
        ]);
        let mut vertices = quad_vertices();
        vertices.extend([
            [0.25, 0.25, 0.0],
            [0.5, 0.25, 0.0],
            [0.5, 0.5, 0.0],
            [0.25, 0.5, 0.0],
        ]);
        let run = Mesh { vertices, ..run };
        let mut hole = quad_ring();
        hole.holes = vec![vec![4, 5, 6, 7]];
        let mut stats = EdgeStats::default();
        let index = RunEdges::new(&run, &visible, &mut stats);
        assert!(matches!(
            index.boundary(&run, &[&hole], &IDENTITY),
            Boundary::MissingRecords(4)
        ));
    }
}
