//! Exact ACIS SAT text writer: B-rep round trip, refusals and topology rings.
//!
//! The round trip is **internal consistency**: our writer builds a
//! `SatDocument`, acadrust serialises it, acadrust parses it back, and this
//! crate lifts it to a neutral B-rep again. No third-party CAD application reads
//! the payload, and nothing here is a vendor-compatibility claim.
//!
//! Export-side contracts (DXF `3DSOLID`, DWG fallback, block-embedded solids)
//! live in `tests::acis_export`.

use super::acis_fixtures::*;
use super::*;

use cad_domain::{BrepCurve, BrepFace, BrepPlacement, BrepShell, BrepSurface};

use crate::export::acis::{brep_to_sat, AcisWriteRefusal};
use crate::sat_to_brep;

// ---------------------------------------------------------------------------
// Round trip: box / wedge / pyramid
// ---------------------------------------------------------------------------

#[test]
fn planar_box_round_trips_with_six_faces_twelve_edges_and_volume() {
    let authored = box_brep(2.0, 3.0, 4.0);
    let parsed = round_trip(&authored);
    assert_brep_equivalent(&authored, &parsed);
    assert_eq!(parsed.face_count(), 6, "a box has six faces");
    let edges: usize = parsed.shells[0]
        .faces
        .iter()
        .flat_map(|face| &face.loops)
        .map(|loop_| loop_.edges.len())
        .sum();
    assert_eq!(edges, 24, "each face has four coedges (12 geometric edges)");

    let volume = tessellated_volume(parsed);
    let expected = 2.0 * 3.0 * 4.0;
    assert!(
        (volume - expected).abs() / expected <= 1e-3,
        "box volume {volume} must match {expected} within 0.1%"
    );
}

#[test]
fn planar_wedge_round_trips_and_keeps_its_half_volume() {
    let authored = wedge_brep(4.0, 2.0, 3.0);
    let parsed = round_trip(&authored);
    assert_brep_equivalent(&authored, &parsed);
    assert_eq!(
        parsed.face_count(),
        5,
        "two triangles plus three rectangles"
    );

    let volume = tessellated_volume(parsed);
    let expected = 4.0 * 2.0 * 3.0 / 2.0;
    assert!(
        (volume - expected).abs() / expected <= 1e-3,
        "wedge volume {volume} must match {expected} within 0.1%"
    );
}

#[test]
fn planar_pyramid_round_trips_and_keeps_its_third_volume() {
    let authored = pyramid_brep(3.0, 4.0);
    let parsed = round_trip(&authored);
    assert_brep_equivalent(&authored, &parsed);
    assert_eq!(
        parsed.face_count(),
        5,
        "one square base plus four triangles"
    );

    let volume = tessellated_volume(parsed);
    let expected = 3.0 * 3.0 * 4.0 / 3.0;
    assert!(
        (volume - expected).abs() / expected <= 1e-3,
        "pyramid volume {volume} must match {expected} within 0.1%"
    );
}

#[test]
fn a_similarity_placement_is_baked_into_points_and_surface_frames() {
    let mut authored = box_brep(2.0, 3.0, 4.0);
    // 90° about Z, uniform scale 2, then a translation. `BrepPlacement::apply`
    // maps (x, y) to `s·(x·m00 + y·m10, x·m01 + y·m11)`.
    authored.placement = Some(BrepPlacement {
        matrix: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        translation: p(10.0, 20.0, 30.0),
        scale: 2.0,
    });
    let parsed = round_trip(&authored);
    assert!(
        parsed.placement.is_none(),
        "the placement is baked, not written"
    );
    assert_eq!(parsed.face_count(), 6);

    // The baked box: x ∈ [10, 10 + 2·3], y ∈ [20 − 2·2, 20], z ∈ [30, 30 + 2·4].
    let (min, max) = brep_bounds(&parsed);
    let expected_min = p(10.0, 16.0, 30.0);
    let expected_max = p(16.0, 20.0, 38.0);
    let tolerance = 1e-9 * diagonal(&authored);
    for (actual, expected, axis) in [
        (min.x, expected_min.x, "min x"),
        (min.y, expected_min.y, "min y"),
        (min.z, expected_min.z, "min z"),
        (max.x, expected_max.x, "max x"),
        (max.y, expected_max.y, "max y"),
        (max.z, expected_max.z, "max z"),
    ] {
        assert!(
            (actual - expected).abs() <= tolerance,
            "baked {axis} is {actual}, expected {expected}"
        );
    }

    // The rotation and translation are preserved, not merely the volume: the
    // authored origin lands on the translation and the +X normal turns to −Y.
    let origin_landed = parsed.shells[0]
        .faces
        .iter()
        .flat_map(|face| &face.loops)
        .flat_map(|loop_| &loop_.edges)
        .any(|curve| matches!(curve, BrepCurve::Line { start, .. } if norm(sub(*start, p(10.0, 20.0, 30.0))) <= tolerance));
    assert!(
        origin_landed,
        "the authored origin is baked onto the placement translation"
    );
    let normals: Vec<Point3> = parsed.shells[0]
        .faces
        .iter()
        .filter_map(|face| match &face.surface {
            BrepSurface::Plane { normal, .. } => Some(*normal),
            _ => None,
        })
        .collect();
    assert!(
        normals
            .iter()
            .any(|normal| norm(sub(*normal, p(0.0, -1.0, 0.0))) <= 1e-12),
        "the +X face normal rotates to -Y: {normals:?}"
    );
    assert!(
        normals
            .iter()
            .any(|normal| norm(sub(*normal, p(0.0, 0.0, 1.0))) <= 1e-12),
        "the +Z face normal is unchanged by a Z rotation: {normals:?}"
    );

    assert_loops_lie_in_their_planes(&parsed, tolerance);
    let volume = tessellated_volume(parsed);
    let expected_volume = 8.0 * 2.0 * 3.0 * 4.0;
    assert!(
        (volume - expected_volume).abs() / expected_volume <= 1e-3,
        "the baked placement preserves the scaled volume: {volume} vs {expected_volume}"
    );
}

// ---------------------------------------------------------------------------
// Refusals: nothing outside the planar subset is fabricated
// ---------------------------------------------------------------------------

#[test]
fn a_cylindrical_face_is_refused_by_name() {
    let cylinder = BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: vec![BrepFace {
                id: 7,
                surface: BrepSurface::Cylinder {
                    origin: p(0.0, 0.0, 0.0),
                    axis: p(0.0, 0.0, 1.0),
                    ref_dir: p(1.0, 0.0, 0.0),
                    radius: 2.0,
                },
                reversed: false,
                loops: Vec::new(),
                tolerance: None,
                domain: None,
            }],
        }],
        placement: None,
        tolerance: None,
    };
    let refusal = brep_to_sat(&cylinder).expect_err("a cylinder is not planar");
    assert_eq!(refusal, AcisWriteRefusal::CurvedFace { face_id: 7 });
    assert_eq!(refusal.code(), "acis.curved_face_not_written");
    assert!(refusal.message().contains("acis.planar_only"));
}

#[test]
fn a_circle_boundary_curve_is_refused_by_name() {
    let mut brep = box_brep(1.0, 1.0, 1.0);
    brep.shells[0].faces[0].loops[0].edges[0] = BrepCurve::Circle {
        center: p(0.5, 0.5, 0.0),
        normal: p(0.0, 0.0, 1.0),
        u_dir: p(1.0, 0.0, 0.0),
        radius: 0.5,
    };
    let refusal = brep_to_sat(&brep).expect_err("a circular boundary is not a straight line");
    assert_eq!(refusal, AcisWriteRefusal::NonLineCurve { face_id: 0 });
    assert_eq!(refusal.code(), "acis.non_line_curve_not_written");
    assert!(refusal.message().contains("acis.planar_only"));
}

#[test]
fn a_two_shell_body_is_refused_instead_of_flattened() {
    let mut brep = box_brep(1.0, 1.0, 1.0);
    let mut inner = box_brep(0.5, 0.5, 0.5);
    brep.shells.push(inner.shells.remove(0));
    let refusal = brep_to_sat(&brep).expect_err("a void needs a second shell");
    assert_eq!(refusal, AcisWriteRefusal::MultipleShells { n: 2 });
    assert_eq!(refusal.code(), "acis.multi_shell_not_written");
}

#[test]
fn an_empty_body_and_a_shelless_body_are_refused() {
    assert_eq!(
        brep_to_sat(&BrepData::default()).expect_err("no shell"),
        AcisWriteRefusal::NoShell
    );
    let shelless = BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: Vec::new(),
        }],
        placement: None,
        tolerance: None,
    };
    assert_eq!(
        brep_to_sat(&shelless).expect_err("a shell without faces"),
        AcisWriteRefusal::NoShell
    );
}

#[test]
fn a_sheared_placement_is_refused_instead_of_baked() {
    let mut brep = box_brep(1.0, 1.0, 1.0);
    brep.placement = Some(BrepPlacement {
        matrix: [[1.0, 0.25, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: p(0.0, 0.0, 0.0),
        scale: 1.0,
    });
    let refusal = brep_to_sat(&brep).expect_err("a shear is not a similarity");
    assert_eq!(refusal, AcisWriteRefusal::Placement);
    assert_eq!(refusal.code(), "acis.placement_not_written");
}

#[test]
fn plane_frames_are_normalised_and_a_skewed_frame_is_refused() {
    // Non-unit but orthogonal: the printed frame is normalised, so the payload
    // always carries a unit normal and u direction.
    let mut scaled = box_brep(1.0, 1.0, 1.0);
    if let BrepSurface::Plane { normal, u_dir, .. } = &mut scaled.shells[0].faces[0].surface {
        *normal = scale(*normal, 7.0);
        *u_dir = scale(*u_dir, 3.0);
    }
    let parsed = round_trip(&scaled);
    let BrepSurface::Plane { normal, u_dir, .. } = &parsed.shells[0].faces[0].surface else {
        panic!("the first face stays planar");
    };
    assert!(
        (norm(*normal) - 1.0).abs() <= 1e-12,
        "the written normal is unit length"
    );
    assert!(
        (norm(*u_dir) - 1.0).abs() <= 1e-12,
        "the written u direction is unit length"
    );
    assert_loops_lie_in_their_planes(&parsed, 1e-9 * diagonal(&scaled));

    // A u direction that is not perpendicular to the normal is a skewed frame:
    // writing it would describe a different plane than the boundary, so refuse.
    let mut skewed = box_brep(1.0, 1.0, 1.0);
    if let BrepSurface::Plane { normal, u_dir, .. } = &mut skewed.shells[0].faces[0].surface {
        *u_dir = unit(add(*u_dir, scale(*normal, 0.5)));
    }
    let refusal = brep_to_sat(&skewed).expect_err("a skewed plane frame is not writable");
    assert_eq!(refusal.code(), "acis.invalid_topology_not_written");
    assert!(
        refusal.message().contains("perpendicular"),
        "{}",
        refusal.message()
    );
}

#[test]
fn a_mirrored_placement_is_refused_instead_of_inverting_the_shell() {
    let mut brep = box_brep(1.0, 1.0, 1.0);
    brep.placement = Some(BrepPlacement {
        matrix: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: p(5.0, 0.0, 0.0),
        scale: 1.0,
    });
    let refusal = brep_to_sat(&brep).expect_err("a mirror would invert the shell");
    assert_eq!(refusal, AcisWriteRefusal::Placement);
    assert_eq!(refusal.code(), "acis.placement_not_written");
    assert!(
        refusal.message().contains("reflection"),
        "{}",
        refusal.message()
    );
}

#[test]
fn a_flipped_face_is_refused_as_inconsistently_oriented() {
    let mut brep = box_brep(1.0, 1.0, 1.0);
    {
        let loop_ = &mut brep.shells[0].faces[0].loops[0];
        loop_.edges.reverse();
        for edge in &mut loop_.edges {
            if let BrepCurve::Line { start, end } = edge {
                std::mem::swap(start, end);
            }
        }
    }
    let refusal = brep_to_sat(&brep).expect_err("a flipped face breaks the shell orientation");
    assert_eq!(refusal.code(), "acis.invalid_topology_not_written");
    assert!(
        refusal.message().contains("inverted"),
        "the per-face invariant names the inverted face: {}",
        refusal.message()
    );
}

#[test]
fn an_open_shell_with_a_once_used_edge_is_refused_as_non_manifold() {
    // Drop one face: the remaining edges of the hole are used once.
    let mut brep = box_brep(1.0, 1.0, 1.0);
    brep.shells[0].faces.pop();
    let refusal = brep_to_sat(&brep).expect_err("an open shell is not a closed solid");
    assert_eq!(refusal.code(), "acis.invalid_topology_not_written");
}

// ---------------------------------------------------------------------------
// Multi-loop faces (the fixture's square hole) and the loop/coedge rings
// ---------------------------------------------------------------------------

/// A `reversed` face carries a surface normal opposite its outward direction; the
/// loop (authored outward-CCW) then winds clockwise about that normal, and the
/// face sense must be written as `reversed` for the shell to stay consistent.
#[test]
fn reversed_faces_round_trip_with_their_sense_and_volume() {
    let mut authored = box_brep(2.0, 3.0, 4.0);
    for face in &mut authored.shells[0].faces {
        if let BrepSurface::Plane { normal, .. } = &mut face.surface {
            *normal = scale(*normal, -1.0);
        }
        face.reversed = true;
    }
    let parsed = round_trip(&authored);
    assert_brep_equivalent(&authored, &parsed);
    assert!(
        parsed.shells[0].faces.iter().all(|face| face.reversed),
        "the reversed sense survives the rewrite"
    );
    let volume = tessellated_volume(parsed);
    let expected = 2.0 * 3.0 * 4.0;
    assert!(
        (volume - expected).abs() / expected <= 1e-3,
        "a fully reversed shell still encloses {expected}, got {volume}"
    );
}

/// A face with an inner (hole) loop must keep both loops through the rewrite,
/// and the rewritten solid must keep the fixture's volume.
#[test]
fn a_face_with_a_square_hole_round_trips_with_both_loops() {
    let sat = include_str!("../../../../../fixtures/acis/box-with-square-hole.sat");
    let document = acadrust::SatDocument::parse(sat).expect("the fixture parses");
    let authored = sat_to_brep(&document);
    assert_eq!(authored.face_count(), 10, "the fixture has ten faces");
    let two_loop_faces = authored.shells[0]
        .faces
        .iter()
        .filter(|face| face.loops.len() == 2)
        .count();
    assert!(
        two_loop_faces >= 1,
        "the fixture has a face with an inner loop"
    );

    let parsed = round_trip(&authored);
    assert_brep_equivalent(&authored, &parsed);
    let parsed_two_loop_faces = parsed.shells[0]
        .faces
        .iter()
        .filter(|face| face.loops.len() == 2)
        .count();
    assert_eq!(
        parsed_two_loop_faces, two_loop_faces,
        "the inner loop survives the rewrite"
    );

    // 10 × 10 × 4 outer box minus a 4 × 4 through-hole: 400 − 64.
    let volume = tessellated_volume(parsed);
    let expected = 10.0 * 10.0 * 4.0 - 4.0 * 4.0 * 4.0;
    assert!(
        (volume - expected).abs() / expected <= 1e-3,
        "the holed box keeps volume {expected}, got {volume}"
    );
}

/// Every written loop is a closed coedge ring, each coedge's `prev` is the
/// inverse of its `next`, and partners are mutual.
#[test]
fn every_written_loop_and_coedge_ring_closes() {
    use acadrust::entities::acis::{SatCoedge, SatLoop, SatPointer};

    let sat = brep_to_sat(&box_brep(2.0, 3.0, 4.0)).expect("the box is writable");
    let text = acadrust::SatWriter::write(&sat);
    let document = acadrust::SatDocument::parse(&text).expect("the payload parses");

    let loops = document.records_of_type("loop");
    assert_eq!(loops.len(), 6, "one loop per face");
    for record in loops {
        let loop_ = SatLoop::from_record(record).expect("a loop record");
        let first = loop_.first_coedge();
        assert!(!first.is_null(), "a loop starts at a coedge");
        let mut pointer = first;
        let mut visited = 0usize;
        loop {
            let coedge_record = document.resolve(pointer).expect("the coedge resolves");
            let coedge = SatCoedge::from_record(coedge_record).expect("a coedge record");
            assert_eq!(
                coedge.owner_loop(),
                SatPointer::new(record.index),
                "the coedge names its own loop"
            );
            let next = document
                .resolve(coedge.next())
                .expect("the next coedge resolves");
            let next = SatCoedge::from_record(next).expect("a coedge record");
            assert_eq!(
                next.prev(),
                pointer,
                "prev is the inverse of next in the ring"
            );
            let partner = document
                .resolve(coedge.partner())
                .expect("the partner resolves");
            let partner = SatCoedge::from_record(partner).expect("a coedge record");
            assert_eq!(
                partner.partner(),
                pointer,
                "partners are mutual across the shared edge"
            );
            assert_eq!(
                partner.edge(),
                coedge.edge(),
                "partners are the two uses of one geometric edge"
            );
            assert_ne!(
                partner.sense(),
                coedge.sense(),
                "partners traverse the shared edge in opposite directions"
            );
            visited += 1;
            pointer = coedge.next();
            if pointer == first {
                break;
            }
            assert!(visited <= 8, "the coedge ring must close");
        }
        assert_eq!(visited, 4, "a box face has four coedges");
    }
}
