//! DXF round-trip of the two MULTILEADER values the 2026-11-12 follow-up made
//! writable.
//!
//! `MLEADEREDIT` can now set (and clear) one leader line's own **path type**
//! (`LEADER_LINE 170` behind the `93` override word bit `1`) and **arrowhead size**
//! (`40` behind bit `16`), and `MLEADER` accepts the **dogleg length/direction**
//! (root `LEADER 40` / `11`) plus the **text attachment pair** (entity `179`/`173`,
//! `CONTEXT_DATA 171`/`174`).
//!
//! This test starts from the exact definitions those two commands store, writes
//! them with the ordinary exporter and reads them back with the ordinary importer:
//! no second writer, no second reader, and each assertion names the stored field
//! rather than restating the derivation.

use super::*;
use crate::tests_multileader_fixtures::{export_one, import_bytes, imported_geometries};

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn at(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// The style snapshot both fixtures start from: straight path, 0.36 dogleg,
/// 0.5 arrowhead.
fn style() -> MLeaderStyleValues {
    MLeaderStyleValues {
        path_type: MLeaderPathType::StraightLineSegments,
        enable_landing: true,
        enable_dogleg: true,
        landing_distance: 0.36,
        landing_gap: 0.09,
        arrowhead_size: 0.5,
        text_height: 0.2,
        text_style: StyleId(0),
        text_style_name: "Standard".into(),
        text_font: None,
        scale_factor: 1.0,
    }
}

/// One root, one line, the text at `landing`.
fn parts(lines: Vec<MLeaderLine>, attachment_point: i16, left_attachment: i16) -> MultiLeaderParts {
    MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: "Edited".into(),
            location: at(10.0, 2.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point,
            left_attachment,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: at(10.0, 2.0),
            dogleg_vector: at(1.0, 0.0),
            dogleg_length: 0.36,
        }],
        lines,
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    }
}

/// One leader line from `arrow` to `dogleg`.
fn line(arrow: Point3, dogleg: Point3) -> MLeaderLine {
    MLeaderLine {
        root_index: 0,
        index: 0,
        points: vec![arrow, dogleg],
        path_type: MLeaderPathType::StraightLineSegments,
        override_flags: 0,
        arrowhead_size: 0.5,
        arrowhead_handle: None,
    }
}

/// The definition `MLEADER DOGLEG 2.5 DOGLEGANGLE 90 ATTACH 1 4` stores: the same
/// creation rule the command calls, with the three typed options.
fn created_with_typed_options() -> MultiLeaderGeometry {
    MultiLeaderGeometry::created_with_options(
        style(),
        "Standard".to_string(),
        2,
        at(0.0, 0.0),
        at(10.0, 2.0),
        "Typed".to_string(),
        MLeaderCreateOptions {
            dogleg_length: Some(2.5),
            dogleg_angle: Some(std::f64::consts::FRAC_PI_2),
            attachment_point: Some(1),
            left_attachment: Some(4),
        },
    )
    .expect("the creation options are usable")
}

#[test]
fn a_created_leader_keeps_its_typed_dogleg_and_attachment_through_dxf() {
    let source = created_with_typed_options();
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(source.clone())));
    let reimported = import_bytes(&bytes);
    let geometries = imported_geometries(&reimported);
    assert_eq!(geometries.len(), 1, "one MULTILEADER is written and read");
    let leader = &geometries[0];

    // The root's own dogleg length (`LEADER 40`) wins over the entity/style value,
    // and the requested direction (`LEADER 11`) is read back verbatim.
    assert_eq!(leader.roots().len(), 1);
    let root = &leader.roots()[0];
    assert!(approx(root.dogleg_length, 2.5));
    assert!(approx(root.dogleg_vector.x, 0.0));
    assert!(approx(root.dogleg_vector.y, 1.0));
    // The derived dogleg start survives too: `landing - direction * length`.
    let start = leader.lines()[0].points[1];
    assert!(approx(start.x, 10.0));
    assert!(approx(start.y, -0.5));

    // The attachment pair is the entity-level one the encoder wrote (179/173), not
    // a re-resolved style value.
    assert_eq!(leader.text().attachment_point, 1);
    assert_eq!(leader.text().left_attachment, 4);

    // No per-line override was invented on the way through the file.
    assert_eq!(leader.lines()[0].override_flags, 0);
    assert!(approx(leader.style().landing_distance, 0.36));
}

#[test]
fn mleaderedit_line_overrides_survive_a_dxf_round_trip() {
    // The state `MLEADEREDIT PATHTYPE 0 SPLINE` + `ARROWSIZE 0 2.5` leaves: the
    // line's own values behind the two override bits.
    let mut overridden = line(at(0.0, 5.0), at(9.0, 5.0));
    overridden.path_type = MLeaderPathType::Spline;
    overridden.arrowhead_size = 2.5;
    overridden.override_flags = MLeaderLine::PATH_TYPE | MLeaderLine::ARROWHEAD_SIZE;
    let (geometry, _) = MultiLeaderGeometry::from_import(parts(vec![overridden], 2, 2));
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(geometry)));
    let geometries = imported_geometries(&import_bytes(&bytes));
    assert_eq!(geometries.len(), 1);
    let stored = &geometries[0].lines()[0];
    assert_eq!(
        stored.override_flags & MLeaderLine::PATH_TYPE,
        MLeaderLine::PATH_TYPE
    );
    assert_eq!(stored.path_type, MLeaderPathType::Spline);
    assert_eq!(
        stored.override_flags & MLeaderLine::ARROWHEAD_SIZE,
        MLeaderLine::ARROWHEAD_SIZE
    );
    assert!(approx(stored.arrowhead_size, 2.5));

    // The state `MLEADEREDIT BYSTYLE 0 PATH|ARROW` leaves: both bits clear, so the
    // entity/style values apply again after the reload. A writer that re-emitted a
    // stale override would fail here.
    let cleared =
        MultiLeaderGeometry::from_import(parts(vec![line(at(0.0, 5.0), at(9.0, 5.0))], 2, 2)).0;
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(cleared)));
    let geometries = imported_geometries(&import_bytes(&bytes));
    assert_eq!(geometries.len(), 1);
    let stored = &geometries[0].lines()[0];
    assert_eq!(stored.override_flags, 0, "no override bit came back");
    assert_eq!(stored.path_type, MLeaderPathType::StraightLineSegments);
    assert!(approx(stored.arrowhead_size, 0.5));
}
