use super::*;
use cad_db::LinetypePattern;

#[test]
fn conic_circle_is_emitted_as_an_sdf_primitive() {
    let registry = ProviderRegistry::with_default_provider();
    let center = p(3.0, -2.0);
    let radius = 7.5;
    let geometry = circle_geometry(center, radius);
    let db = geometry_db(curve_entity(11, "AcDbCircle", geometry));
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(11)).unwrap(), &ctx)
        .unwrap();
    // Slice B flips the emission gate: the circle travels as a Conic primitive
    // for the renderer's SDF draw, not as chord points.
    assert_eq!(stats.produced(), 1, "the circle must travel as a conic");
    assert_eq!(stats.sdf_eligible(), 1);
    let conics = emitted_conics(&rep);
    assert_eq!(conics.len(), 1, "the circle must emit one SDF conic");
    assert!(conics[0].full, "a circle is a full-turn conic");
    assert!(
        emitted_points(&rep).is_empty(),
        "an eligible circle must not be chord-expanded"
    );
}

#[test]
fn conic_arc_is_sdf_and_ellipse_stays_chords() {
    let registry = ProviderRegistry::with_default_provider();
    let arc = arc_geometry(p(1.0, 2.0), 5.0, 0.35, 2.1);
    let ellipse = SemanticGeometry::Ellipse {
        center: p(-4.0, 3.0),
        normal: p3(0.0, 0.0, 1.0),
        major_axis: p3(6.0, 1.5, 0.0),
        ratio: 0.4,
        start: 0.2,
        sweep: 4.0,
    };
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());

    // The arc is slice-1 eligible: it emits an SDF conic, no chords.
    let arc_db = geometry_db(curve_entity(21, "AcDbArc", arc.clone()));
    let arc_rep = registry
        .build_expanded(&arc_db, arc_db.entity(EntityId(21)).unwrap(), &ctx)
        .unwrap();
    assert_eq!(emitted_conics(&arc_rep).len(), 1);
    assert!(!emitted_conics(&arc_rep)[0].full);
    assert!(emitted_points(&arc_rep).is_empty());

    // The ellipse is slice 2: it stays chord-tessellated, matching the old path.
    let ellipse_db = geometry_db(curve_entity(22, "AcDbEllipse", ellipse.clone()));
    let ellipse_rep = registry
        .build_expanded(&ellipse_db, ellipse_db.entity(EntityId(22)).unwrap(), &ctx)
        .unwrap();
    assert!(emitted_conics(&ellipse_rep).is_empty());
    assert_points_close(
        &emitted_points(&ellipse_rep),
        &chord_baseline(&ellipse, &ctx),
        1e-9,
    );

    assert_eq!(stats.produced(), 2);
    assert_eq!(stats.sdf_eligible(), 1, "only the arc is slice-1 eligible");
    assert_eq!(
        stats.sdf_geometric(),
        1,
        "only the arc is geometrically a slice-1 circle arc (the ellipse is not)"
    );
}

#[test]
fn force_chord_hook_routes_an_eligible_circle_to_chords() {
    // The SDF soft-revert lever: `force_chord_conics` sends even an eligible
    // circle through the chord path, matching the pre-SDF draw exactly.
    let registry = ProviderRegistry::with_default_provider();
    let geometry = circle_geometry(p(2.0, -1.0), 6.0);
    let db = geometry_db(curve_entity(12, "AcDbCircle", geometry.clone()));
    let ctx = context().with_force_chord_conics();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(12)).unwrap(), &ctx)
        .unwrap();
    assert!(emitted_conics(&rep).is_empty());
    assert_points_close(
        &emitted_points(&rep),
        &chord_baseline(&geometry, &ctx),
        1e-9,
    );
}

#[test]
fn insert_transformed_circle_matches_the_ellipse_chord_baseline() {
    // Block 20 owns the circle; block 10 inserts it scaled and translated; the
    // model inserts block 10 with a non-uniform scale, a rotation and a
    // translation. The conic must still expand to the correct ellipse chords.
    let registry = ProviderRegistry::with_default_provider();
    let circle = circle_geometry(p(0.0, 0.0), 4.0);
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(20),
        entities: vec![EntityId(200)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut block_circle = curve_entity(200, "AcDbCircle", circle.clone());
    block_circle.space = SpaceId::Block(BlockId(20));
    b.insert_entity(block_circle).unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(10),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    let inner = Transform3::translation(p(1.0, 2.0)).matrix_mul(&Transform3::scale(0.5));
    b.insert_entity(insert_with_transform(
        100,
        SpaceId::Block(BlockId(10)),
        20,
        inner,
    ))
    .unwrap();
    let (s, c) = (
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    );
    let rotation = Transform3::from_basis(p(c, s), p(-s, c), p(0.0, 0.0));
    let nonuniform = Transform3::from_basis(p(2.0, 0.0), p(0.0, 0.5), p(0.0, 0.0));
    let outer = Transform3::translation(p(10.0, -3.0))
        .matrix_mul(&nonuniform)
        .matrix_mul(&rotation);
    b.insert_entity(insert_with_transform(1, SpaceId::Model, 10, outer))
        .unwrap();
    let db = b.finish().unwrap();

    let composed = outer.matrix_mul(&inner);
    let baseline: Vec<Point3> = chord_baseline(&circle, &context())
        .into_iter()
        .map(|point| composed.apply_point(point))
        .collect();
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &ctx)
        .unwrap();
    assert_eq!(stats.produced(), 1);
    assert_points_close(&emitted_points(&rep), &baseline, 1e-9);
}

#[test]
fn zero_axis_conic_falls_back_to_the_chord_path() {
    let registry = ProviderRegistry::with_default_provider();
    let geometry = circle_geometry(p(0.0, 0.0), 0.0);
    let db = geometry_db(curve_entity(31, "AcDbCircle", geometry.clone()));
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(31)).unwrap(), &ctx)
        .unwrap();
    // A zero-radius axis is degenerate: no conic is produced and the legacy
    // chord path is unchanged.
    assert_eq!(stats.produced(), 0);
    assert_points_close(&emitted_points(&rep), &chord_baseline(&geometry, &ctx), 0.0);
}

#[test]
fn non_finite_conic_axis_is_reported_not_silently_tessellated() {
    let registry = ProviderRegistry::with_default_provider();
    let geometry = circle_geometry(p(0.0, 0.0), f64::NAN);
    let db = geometry_db(curve_entity(32, "AcDbCircle", geometry));
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(32)).unwrap(), &ctx)
        .unwrap();
    assert_eq!(stats.produced(), 0);
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.conic_degenerate"));
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
}

#[test]
fn zero_sweep_arc_keeps_the_two_point_chord() {
    let registry = ProviderRegistry::with_default_provider();
    let geometry = arc_geometry(p(1.0, 1.0), 5.0, 0.7, 0.0);
    let db = geometry_db(curve_entity(41, "AcDbArc", geometry.clone()));
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(41)).unwrap(), &ctx)
        .unwrap();
    assert_eq!(stats.produced(), 1);
    let points = emitted_points(&rep);
    assert_eq!(points.len(), 2);
    assert_points_close(&points, &chord_baseline(&geometry, &ctx), 0.0);
}

#[test]
fn full_circle_is_sdf_and_its_chord_baseline_closes() {
    let registry = ProviderRegistry::with_default_provider();
    let geometry = circle_geometry(p(-1.0, 4.0), 3.25);
    let db = geometry_db(curve_entity(42, "AcDbCircle", geometry.clone()));
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(42)).unwrap(), &ctx)
        .unwrap();
    assert_eq!(stats.produced(), 1);
    // A full circle is slice-1 eligible: it travels as an SDF conic, not chords.
    let conics = emitted_conics(&rep);
    assert_eq!(conics.len(), 1);
    assert!(conics[0].full);
    assert!(emitted_points(&rep).is_empty());
    // The chord tessellation (still the ellipse/export path) closes on its first
    // point, as before.
    let points = conics[0].tessellate(crate::provider::chord_tessellation_params(&ctx));
    assert!(points.len() >= 3);
    assert_points_close(&points, &chord_baseline(&geometry, &ctx), 1e-9);
    assert_points_close(&points[points.len() - 1..], &points[..1], 1e-9);
}

#[test]
fn dashed_arc_still_dashes_by_arc_length_through_the_conic_path() {
    // A dashed arc must keep dashing by arc length on the chord path: the conic
    // is expanded to chords first, then subdivided exactly as before.
    let arc = arc_geometry(p(0.0, 0.0), 5.0, 0.0, std::f64::consts::PI);
    let db = line_db(
        curve_entity(51, "AcDbArc", arc),
        EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: LinetypePattern::from_elements([1.0, -1.0]),
            scale: 1.0,
        },
    );
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(51)).unwrap(), &ctx)
        .unwrap();
    assert_eq!(stats.produced(), 1);
    // A dashed conic is not SDF-eligible in this slice, but it is still
    // geometrically a slice-1 circle arc — the split keeps that fact visible.
    assert_eq!(stats.sdf_eligible(), 0);
    assert_eq!(stats.sdf_geometric(), 1);
    let runs = line_primitives(&rep);
    assert!(runs.len() >= 6, "got {} runs", runs.len());
    for run in &runs {
        let mut len = 0.0;
        for w in run.windows(2) {
            len += ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt();
        }
        assert!(
            len <= 1.05,
            "run of {len} units longer than the 1-unit dash"
        );
    }
}
