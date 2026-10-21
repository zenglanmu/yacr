//! `MLeaderText.width` (DXF group 43) becomes
//! [`SemanticGeometry::Text::wrap_width`].
//!
//! The importer already preserves the width and the exporter writes it; without
//! this hop the value was dropped one step before rendering and the annotation
//! never wrapped. The doc child is the only consumer, so the contract is asserted
//! on `display()`.

use cad_domain::{
    MLeaderContent, MLeaderLine, MLeaderPathType, MLeaderReserved, MLeaderRoot, MLeaderStyleValues,
    MLeaderText, MultiLeaderGeometry, MultiLeaderParts, PlaneFrame, Point3, SemanticGeometry,
    StyleId,
};

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn style() -> MLeaderStyleValues {
    MLeaderStyleValues {
        path_type: MLeaderPathType::StraightLineSegments,
        enable_landing: true,
        enable_dogleg: true,
        landing_distance: 2.0,
        landing_gap: 0.1,
        arrowhead_size: 0.5,
        text_height: 2.5,
        text_style: StyleId(0),
        text_style_name: "Standard".into(),
        text_font: Some("arial.ttf".into()),
        scale_factor: 1.0,
    }
}

/// A leader whose annotation text declares DXF group 43 = `width`.
fn leader_with_width(width: f64) -> MultiLeaderGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: "a long annotation that must wrap".into(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 2.5,
            width,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: point(10.0, 5.0),
            dogleg_vector: point(1.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![point(0.0, 0.0), point(8.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    geometry
}

fn display_wrap_width(geometry: &MultiLeaderGeometry) -> Option<f64> {
    geometry
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { wrap_width, .. } => Some(*wrap_width),
            _ => None,
        })
        .expect("the leader display has a text child")
}

#[test]
fn the_stored_width_becomes_the_wrap_boundary() {
    let leader = leader_with_width(4.0);
    assert_eq!(leader.text().width, 4.0, "the definition keeps group 43");
    assert_eq!(
        display_wrap_width(&leader),
        Some(4.0),
        "the display text child carries the boundary to the layout"
    );
}

#[test]
fn a_zero_or_invalid_width_means_no_wrap() {
    // AutoCAD treats 0 as "no width boundary"; negative and non-finite values
    // cannot be a boundary either. The sanitization rule lives in
    // `wrap_width_from_dxf` and is asserted here through the display child.
    for width in [0.0, -3.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let leader = leader_with_width(width);
        assert_eq!(
            display_wrap_width(&leader),
            None,
            "width {width} must not create a boundary"
        );
    }
}

#[test]
fn replacing_the_text_keeps_the_boundary() {
    // `with_text` rebuilds the display from the same `MLeaderText`, so an edit
    // must not silently drop the width and re-flow the annotation.
    let after = leader_with_width(6.0)
        .with_text("short")
        .expect("a non-empty replacement");
    assert_eq!(display_wrap_width(&after), Some(6.0));
}
