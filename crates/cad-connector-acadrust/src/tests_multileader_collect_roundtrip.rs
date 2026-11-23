//! DXF round-trip of a **collected** MULTILEADER.
//!
//! `MLEADERCOLLECT` accepts leaders that disagree on style, `90` override mask
//! and plane frame, and preserves what the format can carry:
//!
//! * per-line path type (`LEADER_LINE 170`) and arrowhead size (`40`) behind the
//!   line's `93` override word, so a collected line keeps its own look;
//! * each root's own dogleg length (`LEADER 40`) and direction (`11`);
//! * a root whose lines were all removed (`MLEADEREDIT REMOVE`) as a `LEADER{}`
//!   record with **no** `LEADER_LINE{}` child.
//!
//! This test writes exactly such a definition with the ordinary exporter and
//! re-reads it with the ordinary importer: no second writer, no second reader.

use super::*;
use crate::tests_multileader_fixtures::{export_one, import_bytes, imported_geometries};

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// The collected leader: base style straight/0.5, one collected line that keeps
/// a spline path and a 2.0 arrowhead, one root already emptied by MLEADEREDIT.
fn collected_geometry() -> MultiLeaderGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 1.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: 0.2,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
            text_angle_type: MLeaderTextAngle::Horizontal,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: "A\\PB".into(),
            location: with_z(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![
            MLeaderRoot {
                leader_index: 0,
                landing_point: with_z(10.0, 5.0),
                dogleg_vector: with_z(1.0, 0.0),
                dogleg_length: 1.0,
            },
            MLeaderRoot {
                leader_index: 1,
                landing_point: with_z(20.0, 0.0),
                dogleg_vector: with_z(-1.0, 0.0),
                dogleg_length: 3.0,
            },
        ],
        // Root 1 has no line at all: that is the state MLEADEREDIT REMOVE leaves.
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![with_z(12.0, 0.0), with_z(19.0, 0.0)],
            path_type: MLeaderPathType::Spline,
            override_flags: MLeaderLine::PATH_TYPE | MLeaderLine::ARROWHEAD_SIZE,
            arrowhead_size: 2.0,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    geometry
}

fn with_z(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn a_collected_leader_with_a_lineless_root_round_trips() {
    let source = collected_geometry();
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(source.clone())));
    let reimported = import_bytes(&bytes);
    let geometries = imported_geometries(&reimported);
    assert_eq!(geometries.len(), 1, "one MULTILEADER is written and read");
    let leader = &geometries[0];

    // The annotation is one string with MTEXT's paragraph break.
    assert_eq!(leader.text().string, "A\\PB");

    // Both roots survive, including the one with no leader line.
    assert_eq!(leader.roots().len(), 2);
    assert_eq!(
        leader.root_lines(1).count(),
        0,
        "the emptied root stays empty"
    );
    assert!(approx(leader.roots()[1].dogleg_length, 3.0));
    assert!(approx(leader.roots()[1].landing_point.x, 20.0));
    assert!(approx(leader.roots()[0].landing_point.x, 10.0));

    // The collected line keeps its own path type and arrowhead size.
    assert_eq!(leader.lines().len(), 1);
    let line = &leader.lines()[0];
    assert_eq!(line.root_index, 0);
    assert_eq!(
        line.override_flags & MLeaderLine::PATH_TYPE,
        MLeaderLine::PATH_TYPE
    );
    assert_eq!(line.path_type, MLeaderPathType::Spline);
    assert_eq!(
        line.override_flags & MLeaderLine::ARROWHEAD_SIZE,
        MLeaderLine::ARROWHEAD_SIZE
    );
    assert!(approx(line.arrowhead_size, 2.0));
    assert!(approx(line.points[0].x, 12.0));
    assert!(approx(line.points[1].x, 19.0));

    // The entity-level (base) snapshot is what a reload resolves for the
    // unmasked values.
    assert_eq!(leader.plane(), PlaneFrame::default());
    assert_eq!(leader.override_mask(), 0);
}
