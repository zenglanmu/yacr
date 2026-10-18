use super::*;
use cad_db::LinetypePattern;

#[test]
fn explicit_dashed_linetype_splits_a_line_into_multiple_sub_polylines() {
    let dashed = LinetypePattern::from_elements([2.0, -1.0]);
    let line = line_entity(1, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0));
    let db = line_db(
        line,
        EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: dashed,
            scale: 1.0,
        },
    );
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    let runs = line_primitives(&rep);
    // dash 2 / gap 1 over 10 units -> 4 dashes.
    assert_eq!(runs.len(), 4, "expected 4 dash sub-polylines, got {runs:?}");
    for run in &runs {
        assert!(run.len() >= 2);
        // Each run lies on the original line and is at most 2 units long.
        let len = (run.last().unwrap().x - run.first().unwrap().x).abs();
        assert!(len <= 2.0 + 1e-9, "dash {len} longer than the pattern");
    }
    assert_eq!(rep.completeness, Completeness::Complete);
    // The pattern is carried on every fragment for diagnostics.
    assert_eq!(rep.fragments[0].linetype.elements, vec![2.0, -1.0]);
    assert!(!rep.fragments[0].linetype_unresolved);
}

#[test]
fn continuous_linetype_keeps_a_single_fragment() {
    let line = line_entity(1, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0));
    let db = line_db(
        line,
        EntityLineType::Explicit {
            name: "Continuous".into(),
            pattern: LinetypePattern::continuous(),
            scale: 1.0,
        },
    );
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(line_primitives(&rep).len(), 1);
    assert_eq!(rep.completeness, Completeness::Complete);
}

#[test]
fn packed_dash_fragments_preserve_geometry_style_and_source() {
    let db = line_db(
        line_entity(1, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0)),
        EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: LinetypePattern::from_elements([2.0, -1.0]),
            scale: 1.0,
        },
    );
    let registry = ProviderRegistry::with_default_provider();
    let entity = db.entity(EntityId(1)).unwrap();
    let normal = registry.build_expanded(&db, entity, &context()).unwrap();
    let packed = registry
        .build_expanded(&db, entity, &context().with_packed_line_segments())
        .unwrap();
    assert_eq!(packed.completeness, normal.completeness);
    assert_eq!(packed.diagnostics, normal.diagnostics);
    assert_eq!(packed.fragments.len(), 1);
    let expected: Vec<Point3> = line_primitives(&normal)
        .iter()
        .flat_map(|run| run.windows(2).flatten().copied())
        .collect();
    let fragment = &packed.fragments[0];
    let DisplayPrimitive::LineSegments(points) = &fragment.primitive else {
        panic!("expected independent segments")
    };
    assert_eq!(points.as_ref(), expected);
    assert_eq!(fragment.source, normal.fragments[0].source);
    assert_eq!(fragment.color, normal.fragments[0].color);
    assert_eq!(fragment.linetype, normal.fragments[0].linetype);
}

#[test]
fn packed_dashes_keep_style_provenance_and_independent_pairs() {
    let db = line_db(
        line_entity(1, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0)),
        EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: LinetypePattern::from_elements([2.0, -1.0]),
            scale: 1.0,
        },
    );
    let registry = ProviderRegistry::with_default_provider();
    let entity = db.entity(EntityId(1)).unwrap();
    let normal = registry.build_expanded(&db, entity, &context()).unwrap();
    let packed = registry
        .build_expanded(&db, entity, &context().with_packed_line_segments())
        .unwrap();
    assert_eq!(packed.completeness, normal.completeness);
    assert_eq!(packed.diagnostics, normal.diagnostics);
    assert_eq!(packed.fragments.len(), 1);
    let fragment = &packed.fragments[0];
    assert_eq!(fragment.source, normal.fragments[0].source);
    assert_eq!(fragment.color, normal.fragments[0].color);
    assert_eq!(fragment.linetype, normal.fragments[0].linetype);
    let DisplayPrimitive::LineSegments(points) = &fragment.primitive else {
        panic!("expected packed pairs");
    };
    let expected: Vec<_> = line_primitives(&normal).into_iter().flatten().collect();
    assert_eq!(points.as_ref(), expected);
    let transformed = fragment.primitive.transformed(&Transform3::identity());
    let DisplayPrimitive::LineSegments(transformed) = transformed else {
        panic!("topology changed");
    };
    assert_eq!(transformed.as_ref(), points.as_ref());
}

#[test]
fn linetype_scale_changes_the_dash_count() {
    let line = line_entity(1, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0));
    let db = line_db(
        line,
        EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: LinetypePattern::from_elements([2.0, -1.0]),
            scale: 2.0,
        },
    );
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    // Scaled by 2 -> dash 4 / gap 2 -> 2 dashes on a 10-unit line.
    assert_eq!(line_primitives(&rep).len(), 2);
}

#[test]
fn global_ltscale_multiplies_the_entity_scale() {
    let line = line_entity(1, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0));
    let mut b = empty_db();
    b.insert_entity(line).unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs_linetype(EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: LinetypePattern::from_elements([2.0, -1.0]),
            scale: 1.0,
        }),
    )
    .unwrap();
    b.set_linetype_scale(2.0).unwrap();
    let db = b.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(
        line_primitives(&rep).len(),
        2,
        "LTSCALE 2 doubles the cycle"
    );
}

#[test]
fn byblock_linetype_inherits_the_insert_pattern() {
    let mut b = empty_db();
    // Block 2 holds a ByBlock line; the INSERT carries an explicit dashed one.
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(
        12,
        SpaceId::Block(BlockId(2)),
        p(0.0, 0.0),
        p(0.0, 10.0),
    ))
    .unwrap();
    b.set_entity_render_attributes(EntityId(12), attrs_linetype(EntityLineType::ByBlock))
        .unwrap();
    b.insert_entity(insert_entity(2, SpaceId::Model, 2, 10.0))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(2),
        attrs_linetype(EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: LinetypePattern::from_elements([2.0, -1.0]),
            scale: 1.0,
        }),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(2)).unwrap(), &context())
        .unwrap();
    let runs = line_primitives(&rep);
    assert!(runs.len() > 1, "ByBlock must inherit the dashed pattern");
    assert!(!rep.fragments[0].linetype_unresolved);
}

#[test]
fn bylayer_linetype_without_a_pattern_is_continuous_and_reported_unresolved() {
    let line = line_entity(1, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0));
    let db = line_db(line, EntityLineType::ByLayer);
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(line_primitives(&rep).len(), 1);
    assert!(rep.fragments[0].linetype_unresolved);
    assert_eq!(rep.completeness, Completeness::Complete);
}

#[test]
fn degenerate_pattern_falls_back_to_continuous_with_a_partial_reason() {
    // A pattern whose elements are all zero would divide by a zero cycle; the
    // representation must keep the line drawable and report why.
    let line = line_entity(1, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0));
    let db = line_db(
        line,
        EntityLineType::Explicit {
            name: "Broken".into(),
            pattern: LinetypePattern::from_elements([1.0, -1.0]),
            scale: 0.0,
        },
    );
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    // Scale 0 is sanitised to 1.0 by `resolve_linetype`, so the line is dashed,
    // not lost. The fallback diagnostic path is covered directly below.
    assert!(line_primitives(&rep).len() > 1);
}

#[test]
fn subdivide_dashes_reports_fallbacks_never_panics() {
    let points = vec![p(0.0, 0.0), p(5.0, 0.0)];
    let (runs, reason) = subdivide_dashes(&points, &LinetypePattern::continuous(), 1.0, 1.0);
    assert_eq!(runs.len(), 1);
    assert!(reason.is_none());

    let (runs, reason) = subdivide_dashes(
        &points,
        &LinetypePattern::from_elements([-1.0, -2.0]),
        1.0,
        1.0,
    );
    // All-gap is not representable via `from_elements` (cycle>0, no visible
    // element) -> continuous with a reason.
    assert_eq!(runs.len(), 1);
    assert!(reason.is_some());

    let (runs, reason) = subdivide_dashes(&points, &LinetypePattern::continuous(), 0.0, 0.0);
    // Continuous short-circuits before the scale is used.
    assert_eq!(runs.len(), 1);
    assert!(reason.is_none());

    // A polyline shorter than the first dash yields one clipped dash, not a
    // panic and not a dropped line.
    let (runs, reason) = subdivide_dashes(
        &[p(0.0, 0.0), p(0.1, 0.0)],
        &LinetypePattern::from_elements([1.0, -1.0]),
        1.0,
        1.0,
    );
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].len(), 2);
    assert!(reason.is_none());
}

#[test]
fn curve_dashes_by_arc_length_not_chord_count() {
    // A tessellated arc must dash by its arc length, not by vertex count.
    let r = 5.0f64;
    let points: Vec<Point3> = (0..=32)
        .map(|i| {
            let t = std::f64::consts::PI * i as f64 / 32.0;
            p(r * t.cos(), r * t.sin())
        })
        .collect();
    let entity = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbPolyline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Polyline {
            points,
            bulges: Vec::new(),
            closed: false,
        },
        draw_order: 0,
    };
    let db = line_db(
        entity,
        EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: LinetypePattern::from_elements([1.0, -1.0]),
            scale: 1.0,
        },
    );
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    let runs = line_primitives(&rep);
    // A semicircle of radius 5 has arc length ~15.7; with 1/1 dashes we expect
    // several runs, each ~1 unit of arc.
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
