//! MULTILEADER **content kinds**: block content and tolerance content are
//! stored, derived into display primitives and refused by name when their
//! payload cannot be drawn.
//!
//! These tests pin the domain half of the feature: the creation rule, the stored
//! payload, and the display derivation each kind produces (the DXF half lives in
//! `cad-connector-acadrust` and the typed command in `cad-app`).

use cad_domain::{
    BlockId, CadError, MLeaderBlockContent, MLeaderContent, MLeaderContentKind,
    MLeaderCreateContent, MLeaderLine, MLeaderPathType, MLeaderReserved, MLeaderRoot,
    MLeaderStyleValues, MLeaderText, MLeaderTextAngle, MLeaderToleranceContent,
    MultiLeaderGeometry, MultiLeaderParts, PlaneFrame, Point3, SemanticGeometry, StyleId,
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
        text_height: 0.2,
        text_style: StyleId(0),
        text_style_name: "Standard".into(),
        text_font: Some("arial.ttf".into()),
        scale_factor: 1.0,
        text_angle_type: MLeaderTextAngle::Horizontal,
    }
}

/// The raw creation result (for the refusal tests).
fn created_result(content: MLeaderCreateContent) -> cad_domain::CadResult<MultiLeaderGeometry> {
    MultiLeaderGeometry::created_with_content(
        style(),
        "Standard".into(),
        2,
        point(0.0, 0.0),
        point(10.0, 5.0),
        content,
        Default::default(),
    )
}

fn created(content: MLeaderCreateContent) -> MultiLeaderGeometry {
    MultiLeaderGeometry::created_with_content(
        style(),
        "Standard".into(),
        2,
        point(0.0, 0.0),
        point(10.0, 5.0),
        content,
        Default::default(),
    )
    .expect("a valid created leader")
}

/// The block instance of a block-content leader's display.
fn block_insert(leader: &MultiLeaderGeometry) -> (BlockId, cad_domain::Transform3) {
    leader
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Insert { block, transform } => Some((*block, *transform)),
            _ => None,
        })
        .expect("block content draws one block instance")
}

/// The stored base point a created block content uses (`base_point` argument of
/// every `MLeaderCreateContent::Block` in these tests unless stated otherwise).
fn origin() -> Point3 {
    point(0.0, 0.0)
}

#[test]
fn the_four_content_kinds_use_the_dxf_codes() {
    for (kind, code) in [
        (MLeaderContentKind::None, 0),
        (MLeaderContentKind::Block, 1),
        (MLeaderContentKind::Text, 2),
        (MLeaderContentKind::Tolerance, 3),
    ] {
        assert_eq!(kind.dxf(), code);
        assert_eq!(MLeaderContentKind::from_dxf(code), Some(kind));
    }
    assert_eq!(MLeaderContentKind::from_dxf(4), None);
    assert_eq!(MLeaderContentKind::from_dxf(-1), None);
}

#[test]
fn block_content_stores_its_identity_and_draws_one_block_instance() {
    let leader = created(MLeaderCreateContent::Block {
        block: BlockId(7),
        name: "Door".into(),
        base_point: Some(origin()),
    });
    assert_eq!(leader.content().kind(), MLeaderContentKind::Block);
    let block = leader.block_content().expect("the block payload");
    assert_eq!(block.block, Some(BlockId(7)));
    assert_eq!(block.name, "Door");
    // Created content carries no source handle and unit placement.
    assert_eq!(block.handle, None);
    assert_eq!(block.location, point(10.0, 5.0));
    assert_eq!(block.rotation, 0.0);
    assert_eq!(block.connection, 0);
    assert_eq!(block.scale.x, 1.0);
    assert_eq!(block.scale.y, 1.0);
    assert_eq!(block.scale.z, 1.0);
    // The annotation string stays empty: a block-content leader has no MText.
    assert_eq!(leader.text().string, "");

    let (block_id, transform) = block_insert(&leader);
    assert_eq!(block_id, BlockId(7));
    // The content point is where the block is placed (the import/export and
    // display composition the pre-P7b compound path already used).
    let placed = transform.apply_point(point(0.0, 0.0));
    assert!((placed.x - 10.0).abs() < 1e-12, "{placed:?}");
    assert!((placed.y - 5.0).abs() < 1e-12, "{placed:?}");
}

#[test]
fn a_non_zero_base_point_lands_on_the_content_point() {
    // The content point receives the block's **base point**, exactly like an
    // `INSERT`: `T(location) · R(rotation) · S(scale) · T(−base)`.
    let base = point(3.0, -2.0);
    let leader = created(MLeaderCreateContent::Block {
        block: BlockId(7),
        name: "Door".into(),
        base_point: Some(base),
    });
    let block = leader.block_content().expect("the payload");
    assert_eq!(block.base_point, Some(base));
    assert_eq!(block.location, point(10.0, 5.0));
    let (_, transform) = block_insert(&leader);
    // The base point is what lands on the content point…
    let placed = transform.apply_point(base);
    assert!((placed.x - 10.0).abs() < 1e-12, "{placed:?}");
    assert!((placed.y - 5.0).abs() < 1e-12, "{placed:?}");
    // …and the definition origin moves by the negated base point.
    let origin_placed = transform.apply_point(origin());
    assert!((origin_placed.x - 7.0).abs() < 1e-12, "{origin_placed:?}");
    assert!((origin_placed.y - 7.0).abs() < 1e-12, "{origin_placed:?}");
}

#[test]
fn the_block_placement_composes_rotation_and_scale_before_the_base_offset() {
    // A stored payload with rotation and non-uniform scale: the linear part is
    // `R · S`, and the translation is `location − R · S · base`.
    let base = point(1.0, 2.0);
    let (leader, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Block(MLeaderBlockContent {
            block: Some(BlockId(7)),
            name: "Door".into(),
            handle: None,
            base_point: Some(base),
            location: point(10.0, 5.0),
            scale: Point3 {
                x: 2.0,
                y: 3.0,
                z: 1.0,
            },
            rotation: std::f64::consts::FRAC_PI_2,
            connection: 0,
        }),
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
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
            points: vec![point(0.0, 0.0), point(8.0, 5.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    let (_, transform) = block_insert(&leader);
    // R(90°) · S(2,3) maps (x, y) to (-3y, 2x).
    assert!(
        (transform.matrix[0][0] - 0.0).abs() < 1e-12,
        "{:?}",
        transform.matrix
    );
    assert!(
        (transform.matrix[0][1] + 3.0).abs() < 1e-12,
        "{:?}",
        transform.matrix
    );
    assert!(
        (transform.matrix[1][0] - 2.0).abs() < 1e-12,
        "{:?}",
        transform.matrix
    );
    assert!(
        (transform.matrix[1][1] - 0.0).abs() < 1e-12,
        "{:?}",
        transform.matrix
    );
    // base (1,2) → (-6, 2); the content point (10,5) places it: t = (16, 3).
    let placed = transform.apply_point(base);
    assert!((placed.x - 10.0).abs() < 1e-12, "{placed:?}");
    assert!((placed.y - 5.0).abs() < 1e-12, "{placed:?}");
    assert!(
        (transform.matrix[0][3] - 16.0).abs() < 1e-12,
        "{:?}",
        transform.matrix
    );
    assert!(
        (transform.matrix[1][3] - 3.0).abs() < 1e-12,
        "{:?}",
        transform.matrix
    );
}

#[test]
fn a_zero_base_point_placement_is_the_identity_base_factor() {
    // The pre-compensation formula was `T(location) · R · S`; with a zero base
    // point the new composition is that same matrix, byte for byte.
    let (leader, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Block(MLeaderBlockContent {
            block: Some(BlockId(7)),
            name: "Door".into(),
            handle: None,
            base_point: Some(origin()),
            location: point(10.0, 5.0),
            scale: Point3 {
                x: 2.0,
                y: 3.0,
                z: 1.0,
            },
            rotation: 0.25,
            connection: 0,
        }),
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
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
            points: vec![point(0.0, 0.0), point(8.0, 5.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    let (_, transform) = block_insert(&leader);
    // The old formula, written out independently.
    let (sin, cos) = 0.25_f64.sin_cos();
    let expected = [
        [cos * 2.0, -sin * 3.0, 0.0, 10.0],
        [sin * 2.0, cos * 3.0, 0.0, 5.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    for row in 0..4 {
        for column in 0..4 {
            assert!(
                (transform.matrix[row][column] - expected[row][column]).abs() < 1e-12,
                "({row}, {column}): {:?} != {expected:?}",
                transform.matrix
            );
        }
    }
}

#[test]
fn block_content_without_a_base_point_is_refused_by_name() {
    let error = created_result(MLeaderCreateContent::Block {
        block: BlockId(7),
        name: "Door".into(),
        base_point: None,
    })
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message)
            if message.contains("mleader.block_metadata_missing")),
        "{error:?}"
    );
}

#[test]
fn an_imported_block_without_a_base_point_draws_nothing_and_says_so() {
    let (leader, reasons) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Block(MLeaderBlockContent {
            block: Some(BlockId(7)),
            name: "Door".into(),
            handle: Some(0x2A),
            base_point: None,
            location: point(10.0, 5.0),
            scale: point(1.0, 1.0),
            rotation: 0.0,
            connection: 0,
        }),
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
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
            points: vec![point(0.0, 0.0), point(8.0, 5.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    assert!(
        !leader
            .display()
            .iter()
            .any(|child| matches!(child, SemanticGeometry::Insert { .. })),
        "a block with no base point is never placed at a guessed origin"
    );
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("no stored base point")),
        "{reasons:?}"
    );
}

#[test]
fn an_unresolved_block_content_keeps_the_kind_and_reports_the_loss() {
    let leader = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Block(MLeaderBlockContent {
            block: None,
            name: "GONE".into(),
            handle: Some(0x2A),
            base_point: None,
            location: point(10.0, 5.0),
            scale: point(1.0, 1.0),
            rotation: 0.0,
            connection: 0,
        }),
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
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
            points: vec![point(0.0, 0.0), point(8.0, 5.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    })
    .0;
    assert_eq!(leader.content().kind(), MLeaderContentKind::Block);
    assert_eq!(leader.block_content().expect("payload").handle, Some(0x2A));
    assert!(
        !leader
            .display()
            .iter()
            .any(|child| matches!(child, SemanticGeometry::Insert { .. })),
        "an unresolved block draws no instance"
    );
}

#[test]
fn tolerance_content_stores_the_frame_and_draws_border_dividers_and_cells() {
    let leader = created(MLeaderCreateContent::Tolerance {
        text: "|0.05|A|B".into(),
    });
    assert_eq!(leader.content().kind(), MLeaderContentKind::Tolerance);
    let tolerance = leader.tolerance_content().expect("the tolerance payload");
    assert_eq!(tolerance.text, "|0.05|A|B");
    // The frame height is the resolved style text height, and the stored
    // annotation height mirrors it (one DXF home: CONTEXT_DATA 41).
    assert!((tolerance.height - 0.2).abs() < 1e-12, "{tolerance:?}");
    assert!((leader.text().height - 0.2).abs() < 1e-12);
    assert_eq!(tolerance.cells(), vec!["0.05", "A", "B"]);

    // The leader path is one polyline, the frame border a second (closed) one;
    // the frame also draws one divider per internal boundary and one cell text.
    let polylines = leader
        .display()
        .iter()
        .filter(|child| matches!(child, SemanticGeometry::Polyline { .. }))
        .count();
    let closed = leader
        .display()
        .iter()
        .filter(|child| matches!(child, SemanticGeometry::Polyline { closed: true, .. }))
        .count();
    let lines = leader
        .display()
        .iter()
        .filter(|child| matches!(child, SemanticGeometry::Line { .. }))
        .count();
    let texts: Vec<&SemanticGeometry> = leader
        .display()
        .iter()
        .filter(|child| matches!(child, SemanticGeometry::Text { .. }))
        .collect();
    assert_eq!(polylines, 2, "the leader path and the frame border");
    assert_eq!(closed, 1, "one closed border rectangle");
    // One dogleg line plus one divider per internal compartment boundary.
    assert_eq!(lines, 3, "the dogleg and two compartment dividers");
    let dividers = leader
        .display()
        .iter()
        .filter(|child| match child {
            // A divider is vertical, full row height, and starts on the frame's
            // bottom edge.
            SemanticGeometry::Line { start, end } => {
                (start.x - end.x).abs() < 1e-12
                    && start.y <= 5.0 + 1e-12
                    && (end.y - start.y - 0.32).abs() < 1e-9
            }
            _ => false,
        })
        .count();
    assert_eq!(dividers, 2, "one divider per internal boundary");
    assert_eq!(texts.len(), 3, "one TEXT per compartment");
    let cells: Vec<String> = texts
        .iter()
        .filter_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(cells, vec!["0.05", "A", "B"]);

    // The border is a closed four-vertex rectangle anchored at the content point.
    let border = leader
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Polyline { points, closed, .. } if *closed => {
                Some((points.clone(), *closed))
            }
            _ => None,
        })
        .expect("the border");
    assert!(border.1, "the border is closed");
    assert_eq!(border.0.len(), 4);
    assert_eq!(border.0[0], point(10.0, 5.0));
    // Every drawn text carries the frame height.
    for child in texts {
        if let SemanticGeometry::Text { height, .. } = child {
            assert!((height - 0.2).abs() < 1e-12, "{height}");
        }
    }
}

#[test]
fn a_tolerance_frame_without_any_compartment_draws_nothing_and_says_so() {
    let leader = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Tolerance(MLeaderToleranceContent {
            text: "||".into(),
            height: 0.2,
        }),
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
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
            points: vec![point(0.0, 0.0), point(8.0, 5.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    assert!(
        !leader
            .0
            .display()
            .iter()
            .any(|child| matches!(child, SemanticGeometry::Text { .. })),
        "no compartment means no cell text"
    );
    assert!(
        leader
            .1
            .iter()
            .any(|reason| reason.contains("tolerance content has no compartment")),
        "{:?}",
        leader.1
    );
}

#[test]
fn tolerance_content_refuses_an_empty_frame_and_a_non_finite_height() {
    let error = MultiLeaderGeometry::created_with_content(
        style(),
        "Standard".into(),
        2,
        point(0.0, 0.0),
        point(10.0, 5.0),
        MLeaderCreateContent::Tolerance { text: "   ".into() },
        Default::default(),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message)
            if message.contains("mleader.tolerance_text_required")),
        "{error:?}"
    );

    let mut bad_style = style();
    bad_style.text_height = f64::NAN;
    let error = MultiLeaderGeometry::created_with_content(
        bad_style,
        "Standard".into(),
        2,
        point(0.0, 0.0),
        point(10.0, 5.0),
        MLeaderCreateContent::Tolerance {
            text: "|0.05|A".into(),
        },
        Default::default(),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message)
            if message.contains("mleader.tolerance_height")),
        "{error:?}"
    );

    // The domain constructor refuses the same payload when it is built directly
    // (the import/creation funnels cannot store an undrawable frame).
    let error = MultiLeaderGeometry::new(MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Tolerance(MLeaderToleranceContent {
            text: "|0.05".into(),
            height: f64::INFINITY,
        }),
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
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
            points: vec![point(0.0, 0.0), point(8.0, 5.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    })
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message)
            if message.contains("mleader.tolerance_height")),
        "{error:?}"
    );
}

#[test]
fn block_content_refuses_a_non_finite_or_collapsing_scale() {
    for scale in [
        point(f64::NAN, 1.0),
        point(0.0, 1.0),
        point(1.0, f64::INFINITY),
    ] {
        let error = MultiLeaderGeometry::created_with_content(
            style(),
            "Standard".into(),
            2,
            point(0.0, 0.0),
            point(10.0, 5.0),
            MLeaderCreateContent::Block {
                block: BlockId(1),
                name: "Door".into(),
                base_point: Some(origin()),
            },
            Default::default(),
        );
        assert!(error.is_ok(), "creation always uses unit scale: {error:?}");

        // The constructor validates a stored payload (the import path).
        let error = MultiLeaderGeometry::new(MultiLeaderParts {
            style: style(),
            style_name: "Standard".into(),
            override_mask: 0,
            content: MLeaderContent::Block(MLeaderBlockContent {
                block: Some(BlockId(1)),
                name: "Door".into(),
                handle: None,
                base_point: Some(origin()),
                location: point(10.0, 5.0),
                scale,
                rotation: 0.0,
                connection: 0,
            }),
            text: MLeaderText {
                string: String::new(),
                location: point(10.0, 5.0),
                rotation: 0.0,
                height: 0.2,
                width: 0.0,
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
                points: vec![point(0.0, 0.0), point(8.0, 5.0)],
                path_type: MLeaderPathType::StraightLineSegments,
                override_flags: 0,
                arrowhead_size: 0.5,
                arrowhead_handle: None,
            }],
            reserved: MLeaderReserved::default(),
            plane: PlaneFrame::default(),
        })
        .unwrap_err();
        assert!(
            matches!(error, CadError::InvalidInput(ref message)
                if message.contains("mleader.block_content_invalid")),
            "{scale:?}: {error:?}"
        );
    }
}

#[test]
fn mtext_and_content_less_creation_are_unchanged_by_the_content_channel() {
    let text = "Note";
    let by_text = MultiLeaderGeometry::created(
        style(),
        "Standard".into(),
        2,
        point(0.0, 0.0),
        point(10.0, 5.0),
        text.into(),
    )
    .expect("the pre-existing rule");
    let by_channel = created(MLeaderCreateContent::Text(text.into()));
    assert_eq!(by_text, by_channel, "MText creation is byte-identical");

    let content_less = MultiLeaderGeometry::created(
        style(),
        "Standard".into(),
        2,
        point(0.0, 0.0),
        point(10.0, 5.0),
        "  ".into(),
    )
    .expect("an empty text is a content-less leader");
    assert_eq!(content_less.content().kind(), MLeaderContentKind::None);
    assert_eq!(
        content_less,
        created(MLeaderCreateContent::Text("  ".into()))
    );

    // A block/tolerance leader has no replaceable text.
    for content in [
        MLeaderCreateContent::Block {
            block: BlockId(1),
            name: "Door".into(),
            base_point: Some(origin()),
        },
        MLeaderCreateContent::Tolerance {
            text: "|0.05".into(),
        },
    ] {
        let leader = created(content);
        let error = leader.with_text("x").unwrap_err();
        assert!(
            matches!(error, CadError::Unsupported(ref message)
                if message.contains("mleader.content_not_text")),
            "{error:?}"
        );
    }
}

#[test]
fn a_rigid_transform_moves_the_block_and_tolerance_content() {
    let leader = created(MLeaderCreateContent::Block {
        block: BlockId(3),
        name: "Door".into(),
        base_point: Some(origin()),
    });
    let moved = leader
        .transformed(&cad_domain::Transform3::translation(point(1.0, 2.0)))
        .expect("a translation is rigid");
    let block = moved.block_content().expect("payload");
    assert_eq!(block.location, point(11.0, 7.0));
    let (_, transform) = block_insert(&moved);
    let placed = transform.apply_point(point(0.0, 0.0));
    assert!((placed.x - 11.0).abs() < 1e-12, "{placed:?}");
    assert!((placed.y - 7.0).abs() < 1e-12, "{placed:?}");

    let tolerance = created(MLeaderCreateContent::Tolerance {
        text: "|0.05".into(),
    });
    let moved = tolerance
        .transformed(&cad_domain::Transform3::translation(point(1.0, 2.0)))
        .expect("a translation is rigid");
    assert_eq!(moved.text().location, point(11.0, 7.0));
    assert_eq!(moved.tolerance_content().expect("payload").height, 0.2);
}
