//! MULTILEADER P7b-1 migration tests (ADR 0019 decision C).
//!
//! Display equivalence against the pre-migration `Compound` output, verbatim
//! field preservation, rigid-transform rebuild, transform refusals and the
//! block/tolerance content split.

use super::*;
use crate::tests_display_dimension_multileader::{
    count_meshes, find_text, has_polyline, has_spline, mleader_with_style, request,
};

// ---- MULTILEADER P7b-1 migration (ADR 0019 decision C) ----

fn approx_value(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn approx_point(a: Point3, b: Point3) -> bool {
    approx_value(a.x, b.x) && approx_value(a.y, b.y) && approx_value(a.z, b.z)
}

fn migrated(geometry: &SemanticGeometry) -> &cad_domain::MultiLeaderGeometry {
    match geometry {
        SemanticGeometry::MultiLeader(multileader) => multileader,
        other => panic!("expected a MultiLeader variant, got {other:?}"),
    }
}

fn points(values: &[(f64, f64, f64)]) -> Vec<Point3> {
    values
        .iter()
        .map(|(x, y, z)| Point3 {
            x: *x,
            y: *y,
            z: *z,
        })
        .collect()
}

/// The migrated spline multileader's display is numerically identical to the
/// pre-P7b `Compound` output captured before the migration.
#[test]
fn migrated_spline_multileader_display_matches_the_pre_p7b_compound() {
    let (doc, ml) = mleader_with_style();
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, completeness) = builder.multileader_semantics(&ml);
    let multileader = migrated(&geometry);
    let display = multileader.display();
    assert_eq!(display.len(), 3, "{display:?}");

    match &display[0] {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            assert_eq!(*degree, 3);
            assert!(knots.is_empty());
            assert!(weights.is_empty());
            assert_eq!(
                control_points,
                &points(&[
                    (0.0, 0.0, 0.0),
                    (1.0, 1.0, 0.0),
                    (3.0, 1.5, 0.0),
                    (5.0, 1.0, 0.0),
                ])
            );
        }
        other => panic!("expected the leader spline, got {other:?}"),
    }

    match &display[1] {
        SemanticGeometry::Mesh(mesh) => {
            assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
            assert!(approx_point(
                mesh.vertices[0],
                points(&[(0.0, 0.0, 0.0)])[0]
            ));
            assert!(approx_point(
                mesh.vertices[1],
                Point3 {
                    x: 0.412_478_955_692_152_7,
                    y: 0.294_627_825_494_394_76,
                    z: 0.0,
                }
            ));
            assert!(approx_point(
                mesh.vertices[2],
                Point3 {
                    x: 0.294_627_825_494_394_76,
                    y: 0.412_478_955_692_152_7,
                    z: 0.0,
                }
            ));
        }
        other => panic!("expected the arrowhead mesh, got {other:?}"),
    }

    match &display[2] {
        SemanticGeometry::Line { start, end } => {
            assert!(approx_point(*start, points(&[(5.0, 1.0, 0.0)])[0]));
            assert!(approx_point(
                *end,
                Point3 {
                    x: 5.339_549_875_050_866,
                    y: 0.915_112_531_237_283_4,
                    z: 0.0,
                }
            ));
        }
        other => panic!("expected the dogleg line, got {other:?}"),
    }

    // The migrated variant is never `Complete`: colour/overrides are resolved but
    // not applied.
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("colour")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

/// The straight-path migration produces the same polyline the pre-P7b importer
/// produced, including the style/tail resolution.
#[test]
fn migrated_straight_multileader_display_matches_the_pre_p7b_polyline() {
    let (doc, mut ml) = mleader_with_style();
    ml.property_override_flags = acadrust::entities::MultiLeaderPropertyOverrideFlags::PATH_TYPE
        | acadrust::entities::MultiLeaderPropertyOverrideFlags::LANDING_GAP;
    ml.path_type = acadrust::entities::MultiLeaderPathType::StraightLineSegments;
    ml.context.landing_gap = 0.35;
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);
    let multileader = migrated(&geometry);

    match &multileader.display()[0] {
        SemanticGeometry::Polyline {
            points: leader,
            bulges,
            closed,
        } => {
            assert_eq!(
                leader,
                &points(&[
                    (0.0, 0.0, 0.0),
                    (1.0, 1.0, 0.0),
                    (3.0, 1.5, 0.0),
                    (5.0, 1.0, 0.0),
                ])
            );
            assert!(bulges.is_empty());
            assert!(!closed);
        }
        other => panic!("expected the leader polyline, got {other:?}"),
    }
    // landing 0.4 - gap 0.35 = 0.05, along the same direction as before.
    match &multileader.display()[2] {
        SemanticGeometry::Line { start, end } => {
            assert!(approx_point(*start, points(&[(5.0, 1.0, 0.0)])[0]));
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            assert!(approx_value((dx * dx + dy * dy).sqrt(), 0.05));
            assert!(approx_value(dy / dx, -0.25));
        }
        other => panic!("expected the dogleg line, got {other:?}"),
    }
}

/// The annotation text keeps the resolved string/location/rotation/height/width
/// and the raw attachment codes, and its display `Text` carries the mapped
/// alignment.
#[test]
fn migrated_text_preserves_its_fields_and_attachment_mapping() {
    let (doc, mut ml) = mleader_with_style();
    ml.context.has_text_contents = true;
    ml.context.text_string = "Note".into();
    ml.context.text_location = acadrust::types::Vector3::new(10.0, 2.0, 3.0);
    ml.context.text_height = 0.25;
    ml.context.text_width = 4.0;
    ml.context.text_rotation = 0.3;
    ml.text_attachment_point = acadrust::entities::TextAttachmentPointType::Right;
    ml.text_left_attachment = acadrust::entities::TextAttachmentType::BottomOfBottomLine;
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);
    let multileader = migrated(&geometry);

    let text = multileader.text();
    assert_eq!(text.string, "Note");
    assert!(approx_point(
        text.location,
        Point3 {
            x: 10.0,
            y: 2.0,
            z: 3.0
        }
    ));
    assert!(approx_value(text.rotation, 0.3));
    assert!(approx_value(text.height, 0.25));
    assert!(approx_value(text.width, 4.0));
    assert_eq!(text.attachment_point, 3);
    assert_eq!(text.left_attachment, 4);
    assert_eq!(text.h_align(), TextAlignH::Right);
    assert_eq!(text.v_align(), TextAlignV::Bottom);

    match find_text(&geometry).expect("text child") {
        SemanticGeometry::Text {
            height,
            rotation,
            position,
            h_align,
            v_align,
            ..
        } => {
            assert!(approx_value(*height, 0.25));
            assert!(approx_value(*rotation, 0.3));
            assert!(approx_point(
                *position,
                Point3 {
                    x: 10.0,
                    y: 2.0,
                    z: 3.0
                }
            ));
            assert_eq!(*h_align, TextAlignH::Right);
            assert_eq!(*v_align, TextAlignV::Bottom);
        }
        other => panic!("expected text, got {other:?}"),
    }
}

/// The raw `90` mask, per-root/per-line indices, per-line overrides, arrowhead
/// fields and the plane frame survive the migration verbatim.
#[test]
fn migration_preserves_mask_indices_line_overrides_and_plane() {
    let (doc, mut ml) = mleader_with_style();
    let mask = acadrust::entities::MultiLeaderPropertyOverrideFlags::PATH_TYPE
        | acadrust::entities::MultiLeaderPropertyOverrideFlags::LANDING_GAP
        | acadrust::entities::MultiLeaderPropertyOverrideFlags::TEXT_HEIGHT;
    ml.property_override_flags = mask;
    ml.context.landing_gap = 0.02;
    ml.context.text_height = 0.5;
    ml.context.base_point = acadrust::types::Vector3::new(1.0, 2.0, 3.0);
    ml.context.base_direction = acadrust::types::Vector3::new(0.0, 1.0, 0.0);
    ml.context.base_vertical = acadrust::types::Vector3::new(-1.0, 0.0, 0.0);
    ml.context.normal_reversed = true;
    let root = &mut ml.context.leader_roots[0];
    root.leader_index = 11;
    root.connection_point = acadrust::types::Vector3::new(8.0, 3.0, 0.0);
    root.direction = acadrust::types::Vector3::new(0.0, 1.0, 0.0);
    root.landing_distance = 0.9;
    let line = &mut root.lines[0];
    line.index = 7;
    line.override_flags = acadrust::entities::LeaderLinePropertyOverrideFlags::PATH_TYPE
        | acadrust::entities::LeaderLinePropertyOverrideFlags::ARROWHEAD_SIZE;
    line.path_type = acadrust::entities::MultiLeaderPathType::Invisible;
    line.arrowhead_size = 0.75;
    line.arrowhead_handle = Some(acadrust::types::Handle::new(42));

    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);
    let multileader = migrated(&geometry);

    assert_eq!(multileader.override_mask(), mask.bits());
    let root = &multileader.roots()[0];
    assert_eq!(root.leader_index, 11);
    assert!(approx_point(
        root.landing_point,
        Point3 {
            x: 8.0,
            y: 3.0,
            z: 0.0
        }
    ));
    assert!(approx_point(
        root.dogleg_vector,
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0
        }
    ));
    assert!(approx_value(root.dogleg_length, 0.9));

    let line = &multileader.lines()[0];
    assert_eq!(line.root_index, 0);
    assert_eq!(line.index, 7);
    assert_eq!(
        line.override_flags,
        (acadrust::entities::LeaderLinePropertyOverrideFlags::PATH_TYPE
            | acadrust::entities::LeaderLinePropertyOverrideFlags::ARROWHEAD_SIZE)
            .bits()
    );
    assert_eq!(line.path_type, MLeaderPathType::Invisible);
    assert!(approx_value(line.arrowhead_size, 0.75));
    assert_eq!(line.arrowhead_handle, Some(42));

    let plane = multileader.plane();
    assert!(approx_point(
        plane.origin,
        Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0
        }
    ));
    assert!(approx_point(
        plane.u,
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0
        }
    ));
    assert!(approx_point(
        plane.v,
        Point3 {
            x: -1.0,
            y: 0.0,
            z: 0.0
        }
    ));
    assert!(plane.normal_reversed);

    // The per-line Invisible override wins over the style's Spline: no path and
    // no arrowhead are drawn for this line.
    assert!(!has_polyline(&geometry));
    assert!(!has_spline(&geometry));
    assert_eq!(count_meshes(&geometry), 0);
}

/// Unmodelled source data is preserved verbatim in `reserved`, once per source
/// group, and makes the entity `Partial` (never `Complete`).
#[test]
fn reserved_bag_keeps_breaks_background_columns_and_line_bits() {
    let (doc, mut ml) = mleader_with_style();
    ml.context.background_fill_enabled = true;
    ml.context.background_scale_factor = 2.0;
    ml.context.column_type = 2;
    ml.context.column_width = 3.0;
    ml.context.column_sizes = vec![1.0, 2.0];
    ml.context.leader_roots[0]
        .break_points
        .push(acadrust::entities::StartEndPointPair::new(
            acadrust::types::Vector3::new(0.5, 0.0, 0.0),
            acadrust::types::Vector3::new(0.75, 0.0, 0.0),
        ));
    {
        let line = &mut ml.context.leader_roots[0].lines[0];
        line.break_points
            .push(acadrust::entities::StartEndPointPair::new(
                acadrust::types::Vector3::new(1.0, 0.0, 0.0),
                acadrust::types::Vector3::new(1.25, 0.0, 0.0),
            ));
        line.break_info_count = 1;
        line.line_color = acadrust::types::Color::Index(3);
        line.line_weight = acadrust::types::LineWeight::W0_50;
        line.line_type_handle = Some(acadrust::types::Handle::new(77));
    }
    // A second line carries complete `break_infos` **and** the legacy mirror:
    // only the complete record is stored, so a source pair is never written
    // twice (ADR 0019 C.2).
    let second_pair = acadrust::entities::StartEndPointPair::new(
        acadrust::types::Vector3::new(2.0, 0.0, 0.0),
        acadrust::types::Vector3::new(2.25, 0.0, 0.0),
    );
    {
        let root = &mut ml.context.leader_roots[0];
        let line = root.create_line(vec![
            acadrust::types::Vector3::new(2.0, 0.0, 0.0),
            acadrust::types::Vector3::new(4.0, 0.0, 0.0),
        ]);
        line.index = 1;
        line.break_info_count = 1;
        line.break_infos
            .push(acadrust::entities::multileader::LeaderLineBreakInfo {
                segment_index: 2,
                break_points: vec![second_pair],
            });
        line.break_points.push(second_pair);
    }
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, completeness) = builder.multileader_semantics(&ml);
    let multileader = migrated(&geometry);
    let reserved = multileader.reserved();

    assert_eq!(reserved.roots.len(), 1);
    assert_eq!(reserved.roots[0].breaks.len(), 1);
    assert_eq!(reserved.roots[0].breaks[0].start.x, 0.5);
    assert_eq!(reserved.lines.len(), 2);
    assert_eq!(reserved.lines[0].legacy_breaks.len(), 1);
    assert!(reserved.lines[0].breaks.is_empty());
    assert_eq!(reserved.lines[0].line_color.index, Some(3));
    assert_eq!(reserved.lines[0].line_type_handle, Some(77));
    assert_eq!(reserved.lines[0].root_index, 0);
    assert_eq!(reserved.lines[0].line_index, 0);
    assert_eq!(reserved.lines[1].breaks.len(), 1);
    assert_eq!(reserved.lines[1].breaks[0].segment_index, 2);
    assert!(reserved.lines[1].legacy_breaks.is_empty());
    assert_eq!(reserved.lines[1].line_index, 1);

    let background = reserved.background.expect("background preserved");
    assert!(background.fill_enabled);
    assert!(approx_value(background.scale_factor, 2.0));
    let columns = reserved.columns.as_ref().expect("columns preserved");
    assert_eq!(columns.column_type, 2);
    assert_eq!(columns.sizes, vec![1.0, 2.0]);

    match completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r.contains("background")),
                "{reasons:?}"
            );
            assert!(reasons.iter().any(|r| r.contains("column")), "{reasons:?}");
            assert!(reasons.iter().any(|r| r.contains("breaks")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

/// A translation moves every leader vertex, text and plane frame, and rebuilds
/// the display.
#[test]
fn multileader_translation_moves_geometry_and_rebuilds_display() {
    let (doc, ml) = mleader_with_style();
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);
    let multileader = migrated(&geometry).clone();
    let translate = Transform3::translation(Point3 {
        x: 10.0,
        y: -3.0,
        z: 2.0,
    });
    let moved = multileader
        .transformed(&translate)
        .expect("translation accepted");

    assert!(approx_point(
        moved.roots()[0].landing_point,
        Point3 {
            x: 19.0,
            y: -3.0,
            z: 2.0
        }
    ));
    assert!(approx_point(
        moved.lines()[0].points[0],
        Point3 {
            x: 10.0,
            y: -3.0,
            z: 2.0
        }
    ));
    assert!(approx_point(
        moved.plane().origin,
        Point3 {
            x: 10.0,
            y: -3.0,
            z: 2.0
        }
    ));
    match &moved.display()[0] {
        SemanticGeometry::Spline { control_points, .. } => {
            assert!(approx_point(
                control_points[0],
                Point3 {
                    x: 10.0,
                    y: -3.0,
                    z: 2.0
                }
            ));
        }
        other => panic!("expected the moved spline, got {other:?}"),
    }
    match &moved.display()[2] {
        SemanticGeometry::Line { start, end } => {
            assert!(approx_point(
                *start,
                Point3 {
                    x: 15.0,
                    y: -2.0,
                    z: 2.0
                }
            ));
            assert!(approx_point(
                *end,
                Point3 {
                    x: 15.339_549_875_050_866,
                    y: -2.084_887_468_762_717,
                    z: 2.0
                }
            ));
        }
        other => panic!("expected the moved dogleg, got {other:?}"),
    }
}

/// A rigid rotation rotates every leader vertex and the display, and adds the
/// same delta to the text rotation.
#[test]
fn multileader_rotation_moves_geometry_and_rebuilds_display() {
    let (doc, mut ml) = mleader_with_style();
    ml.context.has_text_contents = true;
    ml.context.text_string = "Note".into();
    ml.context.text_location = acadrust::types::Vector3::new(10.0, 2.0, 0.0);
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);
    let multileader = migrated(&geometry).clone();
    // 90 degrees counter-clockwise about +Z.
    let rotate = Transform3::from_basis(
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: -1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    let rotated = multileader.transformed(&rotate).expect("rotation accepted");
    assert!(approx_point(
        rotated.roots()[0].landing_point,
        Point3 {
            x: 0.0,
            y: 9.0,
            z: 0.0
        }
    ));
    match &rotated.display()[0] {
        SemanticGeometry::Spline { control_points, .. } => {
            assert!(approx_point(
                control_points[1],
                Point3 {
                    x: -1.0,
                    y: 1.0,
                    z: 0.0
                }
            ));
        }
        other => panic!("expected the rotated spline, got {other:?}"),
    }
    let text = rotated
        .display()
        .iter()
        .find_map(find_text)
        .expect("text child");
    match text {
        SemanticGeometry::Text {
            position, rotation, ..
        } => {
            assert!(approx_point(
                *position,
                Point3 {
                    x: -2.0,
                    y: 10.0,
                    z: 0.0
                }
            ));
            assert!(approx_value(*rotation, std::f64::consts::FRAC_PI_2));
        }
        other => panic!("expected text, got {other:?}"),
    }
}

/// Scaling (uniform or not) and mirroring are refused for the whole entity
/// (ADR 0019 C.5).
#[test]
fn multileader_refuses_scale_mirror_and_non_uniform_transform() {
    let (doc, ml) = mleader_with_style();
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);
    let multileader = migrated(&geometry).clone();

    let uniform = Transform3::scale(2.0);
    assert!(multileader.transformed(&uniform).is_err());
    // A true reflection: `from_basis` derives the Z column, so it would produce
    // a proper rotation here; flip X explicitly instead.
    let mut mirror = Transform3::identity();
    mirror.matrix[0][0] = -1.0;
    assert!(multileader.transformed(&mirror).is_err());
    let non_uniform = Transform3::from_basis(
        Point3 {
            x: 2.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    assert!(multileader.transformed(&non_uniform).is_err());
}

/// Block and tolerance content stay on the pre-P7b `Compound` path, so their
/// rendering does not regress (ADR 0019 C.3).
#[test]
fn block_and_tolerance_content_stay_on_the_compound_path() {
    let (doc, mut ml) = mleader_with_style();
    ml.content_type = acadrust::entities::LeaderContentType::Tolerance;
    ml.context.has_text_contents = true;
    ml.context.text_string = "Tol".into();
    ml.context.text_location = acadrust::types::Vector3::new(10.0, 2.0, 0.0);
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, completeness) = builder.multileader_semantics(&ml);
    assert!(
        matches!(geometry, SemanticGeometry::Compound(_)),
        "tolerance content stays a compound: {geometry:?}"
    );
    assert!(has_spline(&geometry));
    assert!(find_text(&geometry).is_some());
    assert!(matches!(completeness, Completeness::Partial(_)));

    // A block-content multileader also keeps the compound path (the handle does
    // not resolve here, which is reported rather than silently dropped).
    let (doc, mut ml) = mleader_with_style();
    ml.content_type = acadrust::entities::LeaderContentType::Block;
    ml.context.has_block_contents = true;
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, completeness) = builder.multileader_semantics(&ml);
    assert!(
        matches!(geometry, SemanticGeometry::Compound(_)),
        "block content stays a compound: {geometry:?}"
    );
    match completeness {
        Completeness::Partial(reasons) => assert!(
            reasons.iter().any(|r| r.contains("block content")),
            "{reasons:?}"
        ),
        other => panic!("expected Partial, got {other:?}"),
    }
}
