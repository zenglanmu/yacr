use super::*;

#[test]
fn build_expanded_carries_resolved_transparency_and_geometry_source() {
    let mut b = empty_db();
    // Model line 1: explicit 0.5 opacity, surviving from a proxy cache.
    b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs(
            EntityTransparency::Explicit(0.5),
            GeometrySource::ProxyCache,
        ),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();

    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(rep.fragments[0].alpha, 0.5);
    assert_eq!(rep.fragments[0].geometry_source, GeometrySource::ProxyCache);
    assert_eq!(
        rep.fragments[0].precision,
        Precision::Approximate { error_bound: None },
        "proxy-cache geometry is an approximation, not an exact source value"
    );
}

#[test]
fn build_expanded_resolves_byblock_from_the_containing_insert() {
    let mut b = empty_db();
    // Block 2 holds a ByBlock line; the INSERT carries 0.25.
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
        p(0.0, 1.0),
    ))
    .unwrap();
    b.set_entity_render_attributes(
        EntityId(12),
        attrs(EntityTransparency::ByBlock, GeometrySource::Analytic),
    )
    .unwrap();
    b.insert_entity(insert_entity(2, SpaceId::Model, 2, 10.0))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(2),
        attrs(EntityTransparency::Explicit(0.25), GeometrySource::Analytic),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();

    let rep = registry
        .build_expanded(&db, db.entity(EntityId(2)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    // The child has no opacity of its own: it inherits the INSERT's.
    assert_eq!(rep.fragments[0].alpha, 0.25);
}

#[test]
fn byblock_at_the_model_root_falls_back_to_opaque() {
    let mut b = empty_db();
    b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs(EntityTransparency::ByBlock, GeometrySource::Analytic),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments[0].alpha, 1.0);
}

#[test]
fn build_expanded_carries_resolved_color_and_lineweight() {
    let mut b = empty_db();
    b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs_styled(
            cad_db::EntityColor::Explicit([10, 20, 30]),
            cad_db::EntityLineWeight::Explicit(0.5),
        ),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    let fragment = &rep.fragments[0];
    assert!((fragment.color[0] - 10.0 / 255.0).abs() < 1e-6);
    assert!((fragment.color[1] - 20.0 / 255.0).abs() < 1e-6);
    assert!((fragment.color[2] - 30.0 / 255.0).abs() < 1e-6);
    assert!(!fragment.color_unresolved, "explicit colour is resolved");
    assert_eq!(fragment.lineweight, 0.5);
    assert!(!fragment.lineweight_unresolved);
}

#[test]
fn build_expanded_resolves_byblock_color_and_lineweight_from_the_insert() {
    let mut b = empty_db();
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
        p(0.0, 1.0),
    ))
    .unwrap();
    b.set_entity_render_attributes(
        EntityId(12),
        attrs_styled(
            cad_db::EntityColor::ByBlock,
            cad_db::EntityLineWeight::ByBlock,
        ),
    )
    .unwrap();
    b.insert_entity(insert_entity(2, SpaceId::Model, 2, 10.0))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(2),
        attrs_styled(
            cad_db::EntityColor::Explicit([200, 100, 50]),
            cad_db::EntityLineWeight::Explicit(0.8),
        ),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(2)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    let fragment = &rep.fragments[0];
    // The child is fully ByBlock: it inherits the INSERT's colour and weight.
    assert!((fragment.color[0] - 200.0 / 255.0).abs() < 1e-6);
    assert!((fragment.color[1] - 100.0 / 255.0).abs() < 1e-6);
    assert!((fragment.color[2] - 50.0 / 255.0).abs() < 1e-6);
    assert!(!fragment.color_unresolved);
    assert_eq!(fragment.lineweight, 0.8);
    assert!(!fragment.lineweight_unresolved);
}

#[test]
fn byblock_color_at_the_model_root_is_marked_unresolved() {
    let mut b = empty_db();
    b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs_styled(
            cad_db::EntityColor::ByBlock,
            cad_db::EntityLineWeight::ByBlock,
        ),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    let fragment = &rep.fragments[0];
    // The fallback default is used, but the fragment is honest that it was
    // never resolved from a concrete source value.
    assert_eq!(fragment.color, DEFAULT_RENDER_COLOR);
    assert!(fragment.color_unresolved);
    assert_eq!(fragment.lineweight, DEFAULT_LINEWEIGHT_MM);
    assert!(fragment.lineweight_unresolved);
}

#[test]
fn build_without_import_attributes_marks_style_unresolved() {
    // A hand-built database (or the non-expanded `build`) has no import
    // attributes, so the style falls back to the documented defaults and is
    // explicitly marked unresolved rather than claiming a source value.
    let registry = ProviderRegistry::with_default_provider();
    let e = entity(
        7,
        SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(1.0, 0.0),
        },
    );
    let rep = registry.build(&e, &context()).unwrap();
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(rep.fragments[0].color, DEFAULT_RENDER_COLOR);
    assert!(rep.fragments[0].color_unresolved);
    assert!(rep.fragments[0].lineweight_unresolved);
}

/// A hand-built database (or the non-expanded `build`) has no import
/// attributes, so it must stay fully opaque rather than guessing.
#[test]
fn build_without_import_attributes_is_opaque_analytic() {
    let registry = ProviderRegistry::with_default_provider();
    let e = entity(
        7,
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
    let r = registry.build(&e, &context()).unwrap();
    assert_eq!(r.fragments[0].alpha, 1.0);
    assert_eq!(r.fragments[0].geometry_source, GeometrySource::Analytic);
    assert_eq!(r.fragments[0].precision, Precision::Analytic);
}
