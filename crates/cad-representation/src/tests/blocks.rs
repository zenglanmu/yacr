use super::*;

#[test]
fn insert_instances_expand_with_transform_and_instance_path() {
    let mut b = empty_db();
    // Block 2: one line (0,0)-(0,1).
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
    // Block 1: line (0,0)-(1,0) plus a nested insert of block 2 at y=5.
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(11), EntityId(13)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(
        11,
        SpaceId::Block(BlockId(1)),
        p(0.0, 0.0),
        p(1.0, 0.0),
    ))
    .unwrap();
    let mut nested = insert_entity(13, SpaceId::Block(BlockId(1)), 2, 0.0);
    nested.geometry = SemanticGeometry::Insert {
        block: BlockId(2),
        transform: Transform3::translation(p(0.0, 5.0)),
    };
    b.insert_entity(nested).unwrap();
    // Model: a single insert of block 1 at x=10.
    b.insert_entity(insert_entity(1, SpaceId::Model, 1, 10.0))
        .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.completeness, Completeness::Complete);
    assert_eq!(rep.fragments.len(), 2);

    let mut direct = None;
    let mut deep = None;
    for fragment in &rep.fragments {
        match &fragment.primitive {
            DisplayPrimitive::Lines(pts) => match fragment.source.instance.0.len() {
                1 => direct = Some((pts[0], pts[1])),
                2 => deep = Some((pts[0], pts[1])),
                other => panic!("unexpected instance path depth {other}"),
            },
            _ => panic!("expected lines"),
        }
    }
    let (a, z) = direct.expect("direct block line");
    assert_eq!((a.x, a.y), (10.0, 0.0));
    assert_eq!((z.x, z.y), (11.0, 0.0));
    // Nested line shifted by block 1's insert (0,5) then model insert (10,0).
    let (a, z) = deep.expect("nested block line");
    assert_eq!((a.x, a.y), (10.0, 5.0));
    assert_eq!((z.x, z.y), (10.0, 6.0));
}

#[test]
fn missing_block_definition_is_reported_missing() {
    let mut b = empty_db();
    b.insert_entity(insert_entity(1, SpaceId::Model, 99, 0.0))
        .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert!(matches!(rep.completeness, Completeness::Missing(_)));
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.missing_block"));
    assert!(rep.fragments.is_empty());
}

#[test]
fn self_referencing_block_is_cut_with_partial_report() {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(insert_entity(2, SpaceId::Block(BlockId(0)), 0, 1.0))
        .unwrap();
    b.insert_entity(insert_entity(1, SpaceId::Model, 0, 0.0))
        .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.instance_cycle"));
}
