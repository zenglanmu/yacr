//! EXPLODE contract tests: the DIMENSION/MULTILEADER decompositions at the
//! **imported-content** boundaries (the anonymous picture branch and a spline
//! leader path), added in the 2026-11-11 completion batch.
//!
//! Split out of `explode.rs` so both test files stay inside the code-size limit.

use super::explode::{
    dimension_definition, dimension_style, explode, line, model_geometry, select_one, single_app,
    undo_depth,
};
use super::*;

// ---------------------------------------------------------------------------
// The 2026-11-11 completion batch: the DIMENSION/MULTILEADER decompositions at
// the **imported-content** boundaries (the picture branch and a spline leader
// path), which the synthesized/straight fixtures above do not reach.
// ---------------------------------------------------------------------------

#[test]
fn explode_picture_branch_dimension_produces_every_display_member() {
    // An *imported* dimension with an anonymous picture block is the domain's
    // `from_picture` branch: its display members are the block's already-placed
    // geometry, not a synthesized set. EXPLODE must emit exactly those members
    // (never re-synthesize a different display) and delete the source.
    let members = vec![
        line(0.0, 0.0, 10.0, 0.0),
        SemanticGeometry::Text {
            text: "10.00".into(),
            position: point(5.0, -2.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
        },
    ];
    let dimension = DimensionGeometry::from_picture(
        dimension_definition(),
        "Standard".into(),
        dimension_style(),
        None,
        members.clone(),
    )
    .expect("a valid picture-branch dimension");
    let (mut app, mut session) = single_app(
        SemanticGeometry::Dimension(Box::new(dimension)),
        "AcDbDimension",
    );
    select_one(&mut app, &mut session, 1);
    let outcome = explode(&mut app, &mut session).unwrap();
    assert_eq!(
        outcome.objects.len(),
        2,
        "each picture member becomes one entity"
    );
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 2);
    let mut remaining = members;
    for (_, geometry) in &produced {
        let index = remaining
            .iter()
            .position(|member| member == geometry)
            .expect("every picture member is produced verbatim");
        remaining.remove(index);
    }
    assert!(remaining.is_empty(), "no picture member is dropped");
    assert_source_gone_with_one_undo(&app, EntityId(1));
}

#[test]
fn explode_multileader_with_a_spline_leader_path_keeps_the_spline_verbatim() {
    // A MULTILEADER whose path is a smooth spline: the derived display child is the
    // exact stored `SPLINE` (same degree/control points), and EXPLODE produces it as
    // one real SPLINE entity — it is never flattened into line segments.
    let style = MLeaderStyleValues {
        path_type: MLeaderPathType::Spline,
        enable_landing: true,
        enable_dogleg: true,
        landing_distance: 2.0,
        landing_gap: 0.1,
        arrowhead_size: 0.5,
        text_height: 0.2,
        text_style: StyleId(0),
        text_style_name: "Standard".into(),
        text_font: None,
        scale_factor: 1.0,
    };
    let (leader, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style,
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::None,
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 0.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: point(10.0, 0.0),
            dogleg_vector: point(1.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![point(0.0, 0.0), point(5.0, 2.0), point(8.0, 0.0)],
            path_type: MLeaderPathType::Spline,
            override_flags: MLeaderLine::PATH_TYPE,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    let expected_path = leader
        .display()
        .iter()
        .find(|child| matches!(child, SemanticGeometry::Spline { .. }))
        .cloned()
        .expect("the spline path is a display child");
    let (mut app, mut session) = single_app(
        SemanticGeometry::MultiLeader(Box::new(leader)),
        "AcDbMultiLeader",
    );
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();
    let produced = model_geometry(&app);
    assert!(
        produced
            .iter()
            .any(|(_, geometry)| *geometry == expected_path),
        "the spline path survives verbatim: {produced:?}"
    );
    assert!(
        !produced
            .iter()
            .any(|(_, geometry)| matches!(geometry, SemanticGeometry::Polyline { .. })),
        "a spline path is never flattened into a polyline"
    );
    assert_source_gone_with_one_undo(&app, EntityId(1));
}

#[test]
fn explode_multileader_without_content_produces_no_text_entity() {
    // `MLeaderContent::None` leaders carry no annotation: the decomposition must not
    // invent a text entity, and every produced child is still a display child.
    let style = MLeaderStyleValues {
        path_type: MLeaderPathType::StraightLineSegments,
        enable_landing: true,
        enable_dogleg: true,
        landing_distance: 2.0,
        landing_gap: 0.1,
        arrowhead_size: 0.5,
        text_height: 0.2,
        text_style: StyleId(0),
        text_style_name: "Standard".into(),
        text_font: None,
        scale_factor: 1.0,
    };
    let (leader, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style,
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::None,
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 0.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: point(10.0, 0.0),
            dogleg_vector: point(1.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![point(0.0, 0.0), point(5.0, 0.0), point(8.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    let (mut app, mut session) = single_app(
        SemanticGeometry::MultiLeader(Box::new(leader)),
        "AcDbMultiLeader",
    );
    select_one(&mut app, &mut session, 1);
    explode(&mut app, &mut session).unwrap();
    let produced = model_geometry(&app);
    assert!(
        !produced
            .iter()
            .any(|(_, geometry)| matches!(geometry, SemanticGeometry::Text { .. })),
        "no annotation text is invented: {produced:?}"
    );
    assert_source_gone_with_one_undo(&app, EntityId(1));
}

/// Assert the decomposed source is gone and the whole edit is one undo step (the
/// EXPLODE all-or-nothing shape); the produced count is checked by each test.
fn assert_source_gone_with_one_undo(app: &Application, source: EntityId) {
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(source)
            .is_none(),
        "the source is deleted"
    );
    assert_eq!(undo_depth(app), 1, "one transaction / one undo step");
}
