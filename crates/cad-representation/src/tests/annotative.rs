use super::*;

#[test]
fn annotative_text_scales_by_the_active_scale_factor() {
    let registry = ProviderRegistry::with_default_provider();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let e = text_entity(1, 2.5);
    b.insert_entity(e.clone()).unwrap();
    b.set_entity_render_attributes(EntityId(1), annotative_attributes(true))
        .unwrap();
    let db = b.finish().unwrap();

    // The fontless placeholder encodes the effective height through a fixed
    // geometric ratio, so compare the scaled emission against the unscaled one
    // rather than asserting an absolute value.
    let base = emitted_text_height(&registry.build_expanded(&db, &e, &context()).unwrap());
    assert!(base > 0.0);
    let scaled = context().with_annotation_scale(AnnotationScaleRef::new("1:100", 0.01));
    let r = registry.build_expanded(&db, &e, &scaled).unwrap();
    let ratio = emitted_text_height(&r) / base;
    assert!(
        (ratio - 0.01).abs() < 1e-9,
        "annotative text must scale by the active factor, ratio={ratio}"
    );
}

#[test]
fn non_annotative_text_ignores_the_active_scale() {
    let registry = ProviderRegistry::with_default_provider();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let e = text_entity(1, 2.5);
    b.insert_entity(e.clone()).unwrap();
    b.set_entity_render_attributes(EntityId(1), annotative_attributes(false))
        .unwrap();
    let db = b.finish().unwrap();

    let base = emitted_text_height(&registry.build_expanded(&db, &e, &context()).unwrap());
    let scaled = context().with_annotation_scale(AnnotationScaleRef::new("1:100", 0.01));
    let r = registry.build_expanded(&db, &e, &scaled).unwrap();
    assert_eq!(
        emitted_text_height(&r),
        base,
        "non-annotative text must ignore the active scale"
    );
}

#[test]
fn annotative_text_uses_a_per_scale_placement_override() {
    use cad_db::{AnnotativePlacement, AnnotativeScaleOverride};
    let registry = ProviderRegistry::with_default_provider();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let e = text_entity(1, 2.5);
    b.insert_entity(e.clone()).unwrap();
    let mut attrs = annotative_attributes(true);
    attrs.annotative.overrides.push(AnnotativeScaleOverride {
        scale: "1:100".into(),
        placement: AnnotativePlacement {
            position: Point3 {
                x: 9.0,
                y: 8.0,
                z: 0.0,
            },
            rotation: 0.0,
            height: Some(0.05),
        },
    });
    b.set_entity_render_attributes(EntityId(1), attrs).unwrap();
    let db = b.finish().unwrap();

    let scaled = context().with_annotation_scale(AnnotationScaleRef::new("1:100", 0.01));
    let r = registry.build_expanded(&db, &e, &scaled).unwrap();
    let base = emitted_text_height(&registry.build_expanded(&db, &e, &context()).unwrap());
    match &r.fragments[0].primitive {
        DisplayPrimitive::Text { origin, height, .. } => {
            assert!((origin.x - 9.0).abs() < 1e-12);
            assert!((origin.y - 8.0).abs() < 1e-12);
            // The override height (0.05) replaces the base 2.5; the same fixed
            // placeholder ratio applies.
            let expected = base * (0.05 / 2.5);
            assert!(
                (height - expected).abs() < 1e-9,
                "override height {height} != {expected}"
            );
        }
        _ => panic!("expected Text"),
    }
}

#[test]
fn annotative_scale_with_invalid_factor_is_explicit_partial() {
    let registry = ProviderRegistry::with_default_provider();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let e = text_entity(1, 2.5);
    b.insert_entity(e.clone()).unwrap();
    b.set_entity_render_attributes(EntityId(1), annotative_attributes(true))
        .unwrap();
    let db = b.finish().unwrap();

    let base = emitted_text_height(&registry.build_expanded(&db, &e, &context()).unwrap());
    let bad = context().with_annotation_scale(AnnotationScaleRef::new("1:0", 0.0));
    let r = registry.build_expanded(&db, &e, &bad).unwrap();
    assert!(matches!(r.completeness, Completeness::Partial(_)));
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code == "annotative.invalid_scale"));
    // The base geometry is still drawn, not hidden.
    assert_eq!(emitted_text_height(&r), base);
}

#[test]
fn annotative_non_text_geometry_is_explicit_partial() {
    let registry = ProviderRegistry::with_default_provider();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let e = entity(
        1,
        SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
    );
    b.insert_entity(e.clone()).unwrap();
    b.set_entity_render_attributes(EntityId(1), annotative_attributes(true))
        .unwrap();
    let db = b.finish().unwrap();

    let scaled = context().with_annotation_scale(AnnotationScaleRef::new("1:100", 0.01));
    let r = registry.build_expanded(&db, &e, &scaled).unwrap();
    assert!(matches!(r.completeness, Completeness::Partial(_)));
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code == "annotative.unsupported_entity"));
}

#[test]
fn rebuild_annotative_returns_only_annotative_entities() {
    let registry = ProviderRegistry::with_default_provider();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let a = text_entity(1, 2.5);
    let n = text_entity(2, 2.5);
    b.insert_entity(a).unwrap();
    b.insert_entity(n).unwrap();
    b.set_entity_render_attributes(EntityId(1), annotative_attributes(true))
        .unwrap();
    b.set_entity_render_attributes(EntityId(2), annotative_attributes(false))
        .unwrap();
    let db = b.finish().unwrap();

    let rebuilt = registry
        .rebuild_annotative(&db, &context(), AnnotationScaleRef::new("1:100", 0.01))
        .unwrap();
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0].0, EntityId(1));
}
