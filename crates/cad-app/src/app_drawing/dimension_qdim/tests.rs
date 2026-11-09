//! Unit tests for the QDIM planner (the command path is pinned by
//! `tests::drawing::dimension_qdim`).

use super::*;

fn at(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn request(mode: QDimMode) -> QDimCommand {
    QDimCommand {
        mode,
        ..QDimCommand::default()
    }
}

#[test]
fn continuous_chains_consecutive_endpoints_on_one_line() {
    let lines = vec![
        (EntityId(1), at(0.0, 0.0), at(10.0, 0.0)),
        (EntityId(2), at(10.0, 0.0), at(25.0, 0.0)),
    ];
    let definitions = linear_plan(
        &request(QDimMode::Continuous),
        &lines,
        Vec::new(),
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        3.75,
        3.75,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(definitions.len(), 2);
    let mut pairs = Vec::new();
    for definition in &definitions {
        match &definition.points {
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            } => {
                pairs.push((*first, *second));
                assert_eq!(*dimension_line, at(0.0, 3.75));
            }
            other => panic!("expected linear points, got {other:?}"),
        }
    }
    assert_eq!(
        pairs,
        vec![
            (at(0.0, 0.0), at(10.0, 0.0)),
            (at(10.0, 0.0), at(25.0, 0.0))
        ]
    );
}

#[test]
fn baseline_stacks_from_the_common_origin_and_datum_overrides_it() {
    let lines = vec![(EntityId(1), at(0.0, 0.0), at(10.0, 0.0))];
    let definitions = linear_plan(
        &request(QDimMode::Baseline),
        &lines,
        vec![at(25.0, 0.0)],
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(definitions.len(), 2);
    match &definitions[1].points {
        DimensionPoints::Linear {
            first,
            dimension_line,
            ..
        } => {
            assert_eq!(*first, at(0.0, 0.0));
            assert_eq!(*dimension_line, at(0.0, 4.0));
        }
        other => panic!("expected linear points, got {other:?}"),
    }
    let mut with_datum = request(QDimMode::Baseline);
    with_datum.datum = Some(at(5.0, 0.0));
    let definitions = linear_plan(
        &with_datum,
        &lines,
        vec![at(25.0, 0.0)],
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap();
    for definition in &definitions {
        match &definition.points {
            DimensionPoints::Linear { first, .. } => assert_eq!(*first, at(5.0, 0.0)),
            other => panic!("expected linear points, got {other:?}"),
        }
    }
}

#[test]
fn edit_indices_are_validated_and_removed_before_generation() {
    let lines = vec![(EntityId(1), at(0.0, 0.0), at(10.0, 0.0))];
    let mut edited = request(QDimMode::Continuous);
    edited.remove = vec![1];
    let definitions = linear_plan(
        &edited,
        &lines,
        vec![at(25.0, 0.0)],
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(definitions.len(), 1);
    edited.remove = vec![9];
    let error = linear_plan(
        &edited,
        &lines,
        Vec::new(),
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("qdim.edit_range"), "{error}");
}

#[test]
fn intersection_settings_use_pairwise_segment_crossings() {
    let lines = vec![
        (EntityId(1), at(0.0, -5.0), at(0.0, 5.0)),
        (EntityId(2), at(-5.0, 0.0), at(5.0, 0.0)),
    ];
    let mut settings = request(QDimMode::Continuous);
    settings.snap = QDimSnap::Intersections;
    let error = linear_plan(
        &settings,
        &lines,
        Vec::new(),
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap_err();
    // Exactly one crossing cannot make a chain (needs two origins).
    assert!(format!("{error}").contains("qdim.no_targets"), "{error}");
    let parallel = vec![
        (EntityId(1), at(0.0, 0.0), at(10.0, 0.0)),
        (EntityId(2), at(0.0, 5.0), at(10.0, 5.0)),
    ];
    let error = linear_plan(
        &settings,
        &parallel,
        Vec::new(),
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("qdim.no_intersections"),
        "{error}"
    );
}

#[test]
fn ordinate_uses_the_datum_and_the_dominant_axis() {
    let lines = vec![(EntityId(1), at(10.0, 0.0), at(10.0, 20.0))];
    let definitions = linear_plan(
        &request(QDimMode::Ordinate),
        &lines,
        Vec::new(),
        Some(at(0.0, 1.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(definitions.len(), 2);
    assert!(matches!(
        definitions[0].kind,
        DimensionKind::Ordinate { x_ordinate: true }
    ));
    assert!(matches!(
        definitions[1].kind,
        DimensionKind::Ordinate { x_ordinate: false }
    ));
    match &definitions[0].points {
        DimensionPoints::Ordinate { base, .. } => assert_eq!(*base, at(0.0, 0.0)),
        other => panic!("expected ordinate points, got {other:?}"),
    }
}

#[test]
fn coincident_origins_and_off_plane_inputs_are_refused_by_name() {
    let lines = vec![(EntityId(1), at(0.0, 0.0), at(0.0, 0.0))];
    let error = linear_plan(
        &request(QDimMode::Continuous),
        &lines,
        Vec::new(),
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("qdim.no_targets"), "{error}");

    let line = vec![(EntityId(1), at(0.0, 0.0), at(10.0, 0.0))];
    let off_plane = vec![Point3 {
        x: 20.0,
        y: 0.0,
        z: 5.0,
    }];
    let error = linear_plan(
        &request(QDimMode::Continuous),
        &line,
        off_plane,
        Some(at(1.0, 0.0)),
        &cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
            "Standard".into(),
            cad_domain::StyleId(0),
            None,
        ),
        2.0,
        2.0,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("qdim.off_plane"), "{error}");
}
